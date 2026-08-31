/**
 * Helpers for the tests that read source rather than render it.
 *
 * Adapted from `ui/tests/lib/source.ts`. Kept here rather than imported
 * across the seam: these are test-harness conveniences, not shared UI, and
 * `ui/kit` admits only app-neutral runtime code.
 */

import { readdir, readFile } from 'node:fs/promises';

/**
 * Make a slice of source insensitive to how Prettier wrapped it.
 *
 * Structural assertions are written the way the code reads on one line;
 * Prettier owns the wrapping, so a formatter run that pushes an argument onto
 * its own line must not read as the invariant being broken.
 */
export function normalizeSource(source: string): string {
  return source
    .replace(/\s+/g, ' ')
    .replace(/([([])\s+/g, '$1')
    .replace(/\s+([)\]])/g, '$1')
    .replace(/,\s*([)\]}])/g, '$1');
}

/** Every `.ts`/`.tsx` file under `directory`, recursively. */
export async function collectSourceFiles(directory: URL): Promise<URL[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map(async (entry) => {
      const url = new URL(
        entry.name + (entry.isDirectory() ? '/' : ''),
        directory,
      );
      if (entry.isDirectory()) return collectSourceFiles(url);
      return entry.name.endsWith('.ts') || entry.name.endsWith('.tsx')
        ? [url]
        : [];
    }),
  );
  return nested.flat();
}

/** Read a file relative to the tests directory. */
export function readSource(path: string, base: string | URL): Promise<string> {
  return readFile(new URL(path, base), 'utf8');
}

/**
 * Drop comments from a source file.
 *
 * The invariant tests here are about what the code *does*, and a doc comment
 * that names the sink it exists to forbid — "nothing reads `window.__TAURI__`"
 * — must not read as the sink itself. Block comments go entirely; a line goes
 * only when it *starts* with `//`, so a `https://` inside a string is left
 * alone rather than truncating the line it sits on.
 */
export function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .filter((line) => !/^\s*\/\//.test(line))
    .join('\n');
}
