// Checks a packed tarball the way a user meets it: installed into an empty
// project, run through its command, imported as a library — on the Node
// that runs this script, which may be older than the one the toolchain
// needs. The package supports Node 20; nothing else in the repository
// runs there.
//
// usage: node scripts/smoke-package.mjs <tarball.tgz>
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import process from "node:process";

const PACKAGE_NAME = "@kongyo2/similarity-ts";

/** Two files that hold a copy of one function, one interface and one class. */
const FIXTURES = {
  "src/orders.ts": `
export interface OrderRecord {
  id: string;
  total: number;
  createdAt: Date;
}

export function totalOfOrders(orders: OrderRecord[]): number {
  let total = 0;
  for (const order of orders) {
    if (order.total > 0) {
      total += order.total;
    }
  }
  return total;
}

export class OrderCache {
  private entries = new Map<string, OrderRecord>();
  get(id: string): OrderRecord | undefined {
    return this.entries.get(id);
  }
  put(record: OrderRecord): void {
    this.entries.set(record.id, record);
  }
}
`,
  "src/invoices.ts": `
export interface InvoiceRecord {
  id: string;
  total: number;
  createdAt: Date;
}

export function sumOfInvoices(invoices: InvoiceRecord[]): number {
  let sum = 0;
  for (const invoice of invoices) {
    if (invoice.total > 0) {
      sum += invoice.total;
    }
  }
  return sum;
}

export class InvoiceCache {
  private entries = new Map<string, InvoiceRecord>();
  get(id: string): InvoiceRecord | undefined {
    return this.entries.get(id);
  }
  put(record: InvoiceRecord): void {
    this.entries.set(record.id, record);
  }
}
`,
};

/** A check that did not hold; anything else thrown is a bug in this script. */
class SmokeFailure extends Error {}

function check(condition, message) {
  if (!condition) {
    throw new SmokeFailure(message);
  }
}

/** Run a command in `cwd`; its output is returned, not printed. */
function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    // npm and the installed command are `.cmd` shims on Windows.
    shell: process.platform === "win32",
  });
  check(!result.error, `${command} could not be started: ${result.error?.message}`);
  return result;
}

function pairKey(pair) {
  return [pair.left.symbolName, pair.right.symbolName].sort().join(" ~ ");
}

async function smokeTest(tarball) {
  check(existsSync(tarball), `${tarball} does not exist`);
  const project = await mkdtemp(join(tmpdir(), "similarity-ts-smoke-"));
  try {
    return await exercise(tarball, project);
  } finally {
    await rm(project, { recursive: true, force: true });
  }
}

/** Install `tarball` into the empty directory `project` and use it. */
async function exercise(tarball, project) {
  await writeFile(
    join(project, "package.json"),
    JSON.stringify({ name: "smoke", private: true, type: "module" }),
  );
  for (const [relativePath, content] of Object.entries(FIXTURES)) {
    await mkdir(join(project, relativePath, ".."), { recursive: true });
    await writeFile(join(project, relativePath), content);
  }

  const install = run("npm", ["install", "--no-audit", "--no-fund", tarball], project);
  check(install.status === 0, `npm install exited with ${install.status}\n${install.stderr}`);

  // What ships: the build and the documents, not the sources.
  const installed = join(project, "node_modules", ...PACKAGE_NAME.split("/"));
  const shipped = (await readdir(installed)).sort();
  for (const entry of shipped) {
    check(
      ["LICENSE", "README.md", "dist", "node_modules", "package.json"].includes(entry),
      `the package ships \`${entry}\``,
    );
  }
  for (const file of [
    "dist/cli.js",
    "dist/index.js",
    "dist/index.d.ts",
    "dist/wasm/package.json",
    "dist/wasm/similarity_wasm.js",
    "dist/wasm/similarity_wasm_bg.wasm",
  ]) {
    check(existsSync(join(installed, file)), `the package does not ship \`${file}\``);
  }
  const manifest = JSON.parse(await readFile(join(installed, "package.json"), "utf8"));

  // The command, as npm links it.
  const command = join(project, "node_modules", ".bin", "similarity-ts");
  const version = run(command, ["--version"], project);
  check(
    version.status === 0 && version.stdout.trim() === manifest.version,
    `--version printed \`${version.stdout.trim()}\`, the package is ${manifest.version}`,
  );

  const json = run(command, ["src", "--format", "json"], project);
  check(json.status === 0, `the command exited with ${json.status}\n${json.stderr}`);
  const report = JSON.parse(json.stdout);
  check(report.stats.fileCount === 2, `analyzed ${report.stats.fileCount} files, not 2`);
  check(report.warnings.length === 0, `warnings: ${JSON.stringify(report.warnings)}`);
  for (const [mode, expected] of [
    ["functions", "sumOfInvoices ~ totalOfOrders"],
    ["types", "InvoiceRecord ~ OrderRecord"],
    ["classes", "InvoiceCache ~ OrderCache"],
  ]) {
    check(
      report.byMode[mode].some((pair) => pairKey(pair) === expected),
      `${mode} mode did not report ${expected}: ${report.byMode[mode].map(pairKey).join(", ")}`,
    );
  }

  const pretty = run(command, ["src", "--fail-on-duplicates"], project);
  check(pretty.status === 1, `--fail-on-duplicates exited with ${pretty.status}, not 1`);
  check(
    pretty.stdout.includes("=== Function Similarity ===") &&
      pretty.stdout.includes("src/orders.ts:8-16 totalOfOrders"),
    `the pretty report does not show the function pair:\n${pretty.stdout}`,
  );

  // The library, through the package's own entry point.
  await writeFile(
    join(project, "library.mjs"),
    `import { analyzeProject, formatJsonReport } from ${JSON.stringify(PACKAGE_NAME)};
const report = await analyzeProject({ paths: ["src"] });
console.log(JSON.stringify({ pairs: JSON.parse(formatJsonReport(report)).stats.pairCount }));
`,
  );
  const library = run(process.execPath, ["library.mjs"], project);
  check(library.status === 0, `importing the library failed\n${library.stderr}`);
  check(
    JSON.parse(library.stdout).pairs === report.stats.pairCount,
    `the library reports ${library.stdout.trim()}, the command ${report.stats.pairCount} pairs`,
  );

  return (
    `${PACKAGE_NAME}@${manifest.version} on Node ${process.version}, ` +
    `${report.stats.pairCount} pairs in ${report.stats.fileCount} files`
  );
}

const tarballArgument = process.argv[2];
try {
  check(tarballArgument, "no tarball given (usage: node scripts/smoke-package.mjs <tarball.tgz>)");
  console.log(`Package smoke test passed: ${await smokeTest(resolve(tarballArgument))}.`);
} catch (error) {
  if (!(error instanceof SmokeFailure)) {
    throw error;
  }
  console.error(`Package smoke test failed: ${error.message}`);
  process.exitCode = 1;
}
