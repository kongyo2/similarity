/** What was thrown, as text. Anything can be thrown, not only an `Error`. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
