//! Behavioral atoms: what two near-identical trees must agree on to be
//! the same code.
//!
//! Alpha-renaming makes true rename clones exactly equal, so a pair that is
//! near-identical *except* for a couple of behavior-carrying atoms — a
//! different callee, a flipped operator, a different builtin, a `for-of`
//! turned `for-in` — is the "twins that do different things" shape. The
//! atoms of a function are collected once and compared set-wise.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::tree::TreeNode;

#[derive(Debug, Default)]
pub(crate) struct AtomSets {
    /// Value-level atoms: free identifiers, operators, member/call
    /// optionality markers, null, regexes.
    value_atoms: HashSet<String>,
    /// Control-flow atoms: loop kinds and loop jumps.
    control_atoms: HashSet<String>,
    /// Synthetic atoms (`boundary:*`, `fold:*`) carry occurrence COUNTS:
    /// a function reading `a.at(0)` and `b.at(0)` differs from one
    /// reading `a.at(0)` and `b.at(-1)` by one substituted occurrence,
    /// which plain set semantics would misread as a one-sided addition
    /// (`at[0]=0` present on both sides, `at[0]=-1` only on one).
    synthetic_atoms: HashMap<String, usize>,
}

impl AtomSets {
    /// The atoms of one comparison tree.
    pub(crate) fn of(tree: &TreeNode) -> Self {
        // Pre-pass: locals declared with a string-literal initializer
        // (`let trail = "";`) are provably string accumulators, so their
        // folds are direction-sensitive even when the `+` chain itself
        // carries no literal (`trail = trail + segment` vs
        // `trail = segment + trail`).
        let mut string_literal_locals = HashSet::new();
        collect_string_literal_locals(tree, &mut string_literal_locals);
        let mut atoms = Self::default();
        collect_behavioral_atoms(tree, &mut atoms, &string_literal_locals);
        atoms
    }

    fn add_synthetic(&mut self, atom: String) {
        *self.synthetic_atoms.entry(atom).or_insert(0) += 1;
    }

    /// How the two atom sets differ, from `self`'s (the left tree's) side.
    pub(crate) fn difference(&self, right: &AtomSets) -> AtomDifference {
        AtomDifference {
            missing_in_right: self.value_atoms.difference(&right.value_atoms).count()
                + synthetic_missing(&self.synthetic_atoms, &right.synthetic_atoms),
            missing_in_left: right.value_atoms.difference(&self.value_atoms).count()
                + synthetic_missing(&right.synthetic_atoms, &self.synthetic_atoms),
            control: self.control_atoms.symmetric_difference(&right.control_atoms).count(),
        }
    }

    /// The synthetic (`boundary:*`, `fold:*`) atoms, for tests.
    #[cfg(test)]
    pub(crate) fn synthetic_atom_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.synthetic_atoms.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Total occurrences in `left` not covered by `right` (multiset
/// asymmetric difference).
fn synthetic_missing(left: &HashMap<String, usize>, right: &HashMap<String, usize>) -> usize {
    left.iter()
        .map(|(atom, &count)| count.saturating_sub(right.get(atom).copied().unwrap_or(0)))
        .sum()
}

/// Atom counts one tree has and the other lacks (see [`AtomSets::difference`]).
pub(crate) struct AtomDifference {
    pub(crate) missing_in_left: usize,
    pub(crate) missing_in_right: usize,
    pub(crate) control: usize,
}

/// Positional/slicing builtins whose numeric arguments are boundary
/// semantics. `splice` is special-cased in two ways below: its trailing
/// arguments are inserted values (not positions), and its zero-argument
/// form is NOT equivalent to an explicit leading `0`.
fn boundary_builtin(name: &str) -> bool {
    matches!(
        name,
        "at" | "slice"
            | "splice"
            | "charAt"
            | "charCodeAt"
            | "codePointAt"
            | "substring"
            | "substr"
    )
}

/// Labels of locals whose declarator initializer is a string literal.
fn collect_string_literal_locals(node: &TreeNode, locals: &mut HashSet<String>) {
    if node.kind == "VariableDeclarator" && node.children.len() == 2 {
        let target = &node.children[0];
        let init = &node.children[1];
        if target.kind == "BindingIdentifier" && init.kind == "StringLiteral" {
            locals.insert(target.label.clone());
        }
    }
    for child in &node.children {
        collect_string_literal_locals(child, locals);
    }
}

/// Collect behavior-carrying labels: free identifiers (anything
/// alpha-renaming left intact — call targets, property names, globals),
/// operator labels, optional-chaining markers, `null`, regex literals,
/// plus loop kinds and loop jumps as a separate control class.
///
/// Destructuring-pattern KEYS are skipped: `function f({ x, y })` vs
/// `function f({ left, top })` renames the parameter contract the same
/// way renaming two positional parameters does, and the corpus treats
/// those as duplicates.
fn collect_behavioral_atoms(
    node: &TreeNode,
    atoms: &mut AtomSets,
    string_literal_locals: &HashSet<String>,
) {
    let is_value_atom = match node.kind {
        "Identifier" | "BindingIdentifier" | "PrivateIdentifier" => !node.label.starts_with('§'),
        "BinaryExpression"
        | "LogicalExpression"
        | "UnaryExpression"
        | "UpdateExpression"
        | "AssignmentExpression"
        | "RegExpLiteral"
        | "StaticMemberExpression"
        | "ComputedMemberExpression"
        | "PrivateFieldExpression"
        | "CallExpression"
        | "NullLiteral" => true,
        _ => false,
    };
    if is_value_atom {
        atoms.value_atoms.insert(node.label.clone());
    }
    match node.kind {
        "ForOfStatement" | "ForInStatement" | "ForStatement" | "WhileStatement"
        | "DoWhileStatement" | "BreakStatement" | "ContinueStatement" => {
            atoms.control_atoms.insert(node.label.clone());
        }
        _ => {}
    }
    collect_boundary_call_atoms(node, atoms);
    collect_fold_direction_atoms(node, atoms, string_literal_locals);
    let skip_pattern_key = node.kind == "BindingProperty";
    for (index, child) in node.children.iter().enumerate() {
        if skip_pattern_key && index == 0 {
            continue;
        }
        collect_behavioral_atoms(child, atoms, string_literal_locals);
    }
}

/// Positional/slicing builtins whose numeric arguments and arity are
/// boundary semantics, not parameterizable data. `.at(-1)` reads the
/// newest entry while `.at(0)` reads the oldest, and `.slice(0, n)` keeps
/// exactly the head that `.slice(n)` drops — twins one index literal (or
/// one argument) apart do different work, unlike data-literal twins (a
/// table name, a status code) which stay reportable as parameterizable
/// duplicates.
///
/// Three semantic wrinkles are folded in so equivalent spellings emit
/// equal atoms:
///
/// * the zero-argument form of every member except `splice` is the same
///   operation as an explicit leading `0` (`xs.slice()` ⇔ `xs.slice(0)`,
///   `s.charAt()` ⇔ `s.charAt(0)` — omitted arguments coerce to index
///   0), so it normalizes to the explicit form; `splice()` removes
///   nothing while `splice(0)` removes everything, so its arity stays
///   literal.
/// * `substring` swaps its arguments after clamping (`s.substring(0, n)`
///   ⇔ `s.substring(n, 0)`), so its index literals are recorded
///   position-insensitively.
/// * atoms are occurrence-COUNTED (see `AtomSets::synthetic_atoms`): a
///   second call site reading a different boundary must register as a
///   substitution, not vanish into set semantics.
fn collect_boundary_call_atoms(node: &TreeNode, atoms: &mut AtomSets) {
    if node.kind != "CallExpression" {
        return;
    }
    let Some(callee) = node.children.first() else {
        return;
    };
    if callee.children.len() != 2 {
        return;
    }
    let property = &callee.children[1];
    let name = match callee.kind {
        // `xs.slice(…)`
        "StaticMemberExpression" if property.kind == "Identifier" => property.label.as_str(),
        // `xs["slice"](…)` — the same property access spelled with
        // bracket notation (StringLiteral labels carry their quotes).
        "ComputedMemberExpression" if property.kind == "StringLiteral" => {
            property.label.trim_start_matches('"').trim_end_matches('"')
        }
        _ => return,
    };
    if !boundary_builtin(name) {
        return;
    }
    // Explicit trailing `undefined` is the omitted-argument spelling for
    // these APIs (`xs.slice(0, undefined)` copies to the end exactly like
    // `xs.slice(0)`), so it is trimmed before arity is recorded, and
    // arguments beyond the API's meaningful parameter count are ignored
    // by JavaScript (`xs.at(0, traceId)` reads the same element as
    // `xs.at(0)`), so the recorded arity clamps to that count. `splice`
    // is excluded from both: `splice(0, undefined)` coerces deleteCount
    // to 0 and removes nothing, and its trailing arguments are inserted
    // values, not ignored extras.
    let mut arguments: &[Rc<TreeNode>] = &node.children[1..];
    if name != "splice" {
        while let Some(last) = arguments.last() {
            if last.kind == "Identifier" && last.label == "undefined" {
                arguments = &arguments[..arguments.len() - 1];
            } else {
                break;
            }
        }
        let meaningful_arity = match name {
            "at" | "charAt" | "charCodeAt" | "codePointAt" => 1,
            _ => 2, // slice | substring | substr
        };
        if arguments.len() > meaningful_arity {
            arguments = &arguments[..meaningful_arity];
        }
    }
    let arity = arguments.len();
    if arity == 0 && name != "splice" {
        // Omitted-argument form: same operation as an explicit index 0.
        atoms.add_synthetic(format!("boundary:{name}/1"));
        atoms.add_synthetic(boundary_literal_atom(name, 0, "0"));
        return;
    }
    atoms.add_synthetic(format!("boundary:{name}/{arity}"));
    // `splice` is the one member of the family whose trailing arguments
    // are inserted VALUES, not positions: only `start` and `deleteCount`
    // (positions 0 and 1) are boundaries, and twins differing in an
    // inserted value are ordinary data-literal duplicates.
    let boundary_positions = if name == "splice" { 2 } else { usize::MAX };
    for (position, argument) in arguments.iter().enumerate() {
        if position >= boundary_positions {
            break;
        }
        if let Some(index_literal) = static_index_literal(argument) {
            atoms.add_synthetic(boundary_literal_atom(name, position, &index_literal));
        }
    }
}

/// Atom key for one boundary index literal. `substring` drops the
/// argument position from the key because JavaScript reorders its
/// arguments after clamping — `(0, n)` and `(n, 0)` select the same
/// range — and clamps negatives to 0 first, so `substring(-1, n)` is
/// `substring(0, n)`; only the clamped multiset of literals is
/// behavioral. `slice`/`at` keep negatives distinct (they index from the
/// end) and stay position-keyed.
fn boundary_literal_atom(name: &str, position: usize, literal: &str) -> String {
    if name == "substring" {
        let clamped = if literal.starts_with('-') { "0" } else { literal };
        format!("boundary:{name}[]={clamped}")
    } else {
        format!("boundary:{name}[{position}]={literal}")
    }
}

/// `0`, `1`, `-1`, … — a literal (possibly negated) numeric index.
fn static_index_literal(node: &TreeNode) -> Option<String> {
    if node.kind == "NumericLiteral" {
        return Some(node.label.clone());
    }
    if node.kind == "UnaryExpression" && node.label == "UnaryNegation" && node.children.len() == 1 {
        let inner = &node.children[0];
        if inner.kind == "NumericLiteral" {
            return Some(format!("-{}", inner.label));
        }
    }
    None
}

/// Fold-direction atoms: an assignment that rebuilds its own target from a
/// `+` chain is order-sensitive because string `+` is not commutative —
/// `trail = trail + seg + "/"` appends while `trail = seg + "/" + trail`
/// prepends, and the two loops produce reversed outputs. The accumulator's
/// position in the chain (head / mid / tail) is therefore a behavioral
/// atom. Two deliberate scope limits:
///
/// * the atom only fires when a chain operand is a string LITERAL —
///   that's the one case where the concatenation (and hence its
///   direction) is provable from syntax. Numeric folds (`sum += n` vs
///   `sum = n + sum`) are commutative and must not be penalized, and an
///   untyped identifier chain could be either, so it gets no atom.
/// * the target may be a local (`trail`) or a member slot
///   (`this.trail`, `state.trail`) — member accumulators compare
///   structurally via the same node hash the statement-permutation
///   check uses.
///
/// The canonicalizer already contracts `x = x + y` onto the `+=` shape
/// (an `AssignmentExpression` labeled `Addition`), which is by
/// construction a head fold; non-`+` operators are left alone — their
/// swapped-operand twins already diverge structurally through the
/// compound-contraction asymmetry.
fn collect_fold_direction_atoms(
    node: &TreeNode,
    atoms: &mut AtomSets,
    string_literal_locals: &HashSet<String>,
) {
    if node.kind != "AssignmentExpression" || node.children.len() != 2 {
        return;
    }
    let target = &node.children[0];
    if !matches!(
        target.kind,
        "Identifier"
            | "BindingIdentifier"
            | "StaticMemberExpression"
            | "PrivateFieldExpression"
            | "ComputedMemberExpression"
    ) {
        return;
    }
    // String evidence: a literal operand in the chain proves the
    // concatenation from the expression itself; a target declared with a
    // string-literal initializer (`let trail = "";`) proves it from the
    // declaration. Either way the fold direction is behavioral.
    let target_is_string_local = matches!(target.kind, "Identifier" | "BindingIdentifier")
        && string_literal_locals.contains(&target.label);
    let chain = &node.children[1];
    let mut leaves = Vec::new();
    if node.label == "Addition" {
        // Contracted `x += y` / `x = x + y`: the accumulator is the chain
        // head; the flattened RHS supplies the in-chain string evidence.
        flatten_addition_chain(chain, &mut leaves);
        if target_is_string_local || leaves.iter().any(|leaf| leaf.kind == "StringLiteral") {
            atoms.add_synthetic("fold:head".to_string());
        }
        return;
    }
    if node.label != "Assign" {
        return;
    }
    if chain.kind != "BinaryExpression" || chain.label != "Addition" {
        return;
    }
    flatten_addition_chain(chain, &mut leaves);
    if !target_is_string_local && !leaves.iter().any(|leaf| leaf.kind == "StringLiteral") {
        return;
    }
    let target_hash = target.structural_hash();
    for (position, leaf) in leaves.iter().enumerate() {
        if leaf.kind == target.kind && leaf.structural_hash() == target_hash {
            let marker = if position == 0 {
                "fold:head"
            } else if position == leaves.len() - 1 {
                "fold:tail"
            } else {
                "fold:mid"
            };
            atoms.add_synthetic(marker.to_string());
        }
    }
}

/// Collect the operand leaves of a left/right-nested `+` chain in source
/// order.
fn flatten_addition_chain<'a>(node: &'a TreeNode, leaves: &mut Vec<&'a TreeNode>) {
    if node.kind == "BinaryExpression" && node.label == "Addition" {
        for child in &node.children {
            flatten_addition_chain(child, leaves);
        }
    } else {
        leaves.push(node);
    }
}

/// The top-level statements of a comparison fragment's function body,
/// with the structural hash of each.
pub(crate) struct BodyStatements {
    statements: Vec<Rc<TreeNode>>,
    hashes: Vec<u64>,
}

impl BodyStatements {
    /// `None` when the fragment has no function body at its top level (the
    /// class-wrapped fragments of accessors, static methods and
    /// constructors).
    pub(crate) fn of(tree: &TreeNode) -> Option<Self> {
        let function = tree
            .children
            .iter()
            .find(|child| matches!(child.kind, "FunctionDeclaration" | "ClassDeclaration"))?;
        let block = function.children.iter().find(|child| child.kind == "BlockStatement")?;
        Some(Self {
            statements: block.children.clone(),
            hashes: block.children.iter().map(|statement| statement.structural_hash()).collect(),
        })
    }
}

/// Whether the two fragments' top-level body statements are the same
/// multiset in a different order, AND at least one out-of-order pair
/// touches a shared identifier (a data dependency — reordering those is
/// behavioral; reordering independent statements is style).
pub(crate) fn dependent_statements_permuted(
    left: Option<&BodyStatements>,
    right: Option<&BodyStatements>,
) -> bool {
    fn identifier_labels(node: &TreeNode, labels: &mut HashSet<String>) {
        if matches!(node.kind, "Identifier" | "BindingIdentifier") {
            labels.insert(node.label.clone());
        }
        for child in &node.children {
            identifier_labels(child, labels);
        }
    }

    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    let (hashes1, hashes2) = (&left.hashes, &right.hashes);
    if hashes1.len() < 2 || hashes1.len() != hashes2.len() {
        return false;
    }
    if hashes1 == hashes2 {
        return false;
    }
    let mut sorted1 = hashes1.clone();
    let mut sorted2 = hashes2.clone();
    sorted1.sort_unstable();
    sorted2.sort_unstable();
    if sorted1 != sorted2 {
        return false;
    }

    // Same multiset, different order: find statement pairs whose relative
    // order flipped and check whether any such pair shares an identifier.
    let mut position_in_2: HashMap<u64, Vec<usize>> = HashMap::new();
    for (position, hash) in hashes2.iter().enumerate() {
        position_in_2.entry(*hash).or_default().push(position);
    }
    // Map each statement of side 1 to a position on side 2 (first unused
    // occurrence for duplicate hashes).
    let mut used: HashMap<u64, usize> = HashMap::new();
    let mapped: Vec<usize> = hashes1
        .iter()
        .map(|hash| {
            let index = used.entry(*hash).or_insert(0);
            let positions = &position_in_2[hash];
            let position = positions[(*index).min(positions.len() - 1)];
            *index += 1;
            position
        })
        .collect();

    let labels: Vec<HashSet<String>> = left
        .statements
        .iter()
        .map(|statement| {
            let mut set = HashSet::new();
            identifier_labels(statement, &mut set);
            set
        })
        .collect();

    for a in 0..mapped.len() {
        for b in (a + 1)..mapped.len() {
            if mapped[a] > mapped[b] && !labels[a].is_disjoint(&labels[b]) {
                return true;
            }
        }
    }
    false
}
