#!/usr/bin/env node
/**
 * Assemble dev/app-foks.html from dev/foks-mock-parts/*.
 *
 * Parts are concatenated in filename order. `*.html` parts are pasted as-is;
 * `*.js` parts are pasted inside the single <script> that 00-head.html opens
 * and 99-tail.html closes. Run from the repo root:
 *
 *   node dev/foks-mock-tools/assemble.mjs
 */
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const PARTS = resolve(here, '../foks-mock-parts');
const OUT = resolve(here, '../app-foks.html');

const names = (await readdir(PARTS))
  .filter((n) => /\.(html|js)$/.test(n))
  .sort();
let out = '';
for (const name of names) {
  const text = await readFile(resolve(PARTS, name), 'utf8');
  if (name.endsWith('.js')) {
    out += `\n  /* ======================================================== ${name} */\n`;
    out += text.replace(/\s+$/, '') + '\n';
  } else {
    out += text.replace(/\s+$/, '') + '\n';
  }
}
await writeFile(OUT, out);
console.log(
  `assembled ${names.length} parts into ${OUT} (${out.split('\n').length} lines)`,
);
