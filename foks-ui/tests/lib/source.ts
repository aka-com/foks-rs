/**
 * Static analysis utility functions for source inspection tests.
 */

import { readdir, readFile } from 'node:fs/promises';

/**
 * Normalizes whitespace and bracket spacing in source code strings to facilitate
 * formatting-invariant structural assertions.
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
 * Strips block comments and leading single-line comments from source code
 * so that static analysis assertions match executable code rather than comments.
 */
export function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split('\n')
    .filter((line) => !/^\s*\/\//.test(line))
    .join('\n');
}
