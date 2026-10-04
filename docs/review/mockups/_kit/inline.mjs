#!/usr/bin/env node
// Inline the mockup kit into standalone pages.
//
//   node docs/review/mockups/_kit/inline.mjs page.html [more.html ...]
//   node docs/review/mockups/_kit/inline.mjs --check page.html ...
//
// In each page, the text between /* KIT:CSS:BEGIN */ and /* KIT:CSS:END */
// inside a <style> element is replaced with kit.css, and the text between
// /* KIT:JS:BEGIN */ and /* KIT:JS:END */ inside a <script> element with
// kit.js. The markers stay, so re-running refreshes the copy. --check reports
// pages whose inlined copy is stale and exits 1 without writing.

import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const kitDir = new URL('.', import.meta.url);
const css = readFileSync(new URL('kit.css', kitDir), 'utf8').trim();
const js = readFileSync(new URL('kit.js', kitDir), 'utf8').trim();

if (/<\/style/i.test(css))
  throw new Error('kit.css must not contain "</style"');
// A literal "</script" would end the inline script early.
const safeJs = js.replace(/<\/script/gi, '<\\/script');

const BLOCKS = [
  {
    name: 'CSS',
    element: 'style',
    begin: '/* KIT:CSS:BEGIN */',
    end: '/* KIT:CSS:END */',
    body: css,
  },
  {
    name: 'JS',
    element: 'script',
    begin: '/* KIT:JS:BEGIN */',
    end: '/* KIT:JS:END */',
    body: safeJs,
  },
];

function replaceBlock(html, block, file) {
  const a = html.indexOf(block.begin);
  if (a < 0) throw new Error(`${file}: missing ${block.begin}`);
  if (html.indexOf(block.begin, a + 1) >= 0)
    throw new Error(`${file}: ${block.begin} appears twice`);
  const b = html.indexOf(block.end, a);
  if (b < 0)
    throw new Error(`${file}: missing ${block.end} after ${block.begin}`);
  // The markers must sit inside one <style> or <script> element.
  const open = html.lastIndexOf(`<${block.element}`, a);
  const closeBefore = html.lastIndexOf(`</${block.element}`, a);
  const close = html.indexOf(`</${block.element}`, a);
  if (open < 0 || closeBefore > open || close < b) {
    throw new Error(
      `${file}: ${block.name} markers must be inside a <${block.element}> element`,
    );
  }
  const head = html.slice(0, a + block.begin.length);
  const tail = html.slice(b);
  return `${head}\n${block.body}\n${tail}`;
}

const args = process.argv.slice(2);
const check = args[0] === '--check';
const files = check ? args.slice(1) : args;
if (!files.length) {
  console.error('usage: node inline.mjs [--check] <page.html> [...]');
  process.exit(2);
}

let stale = 0;
for (const file of files) {
  const path = resolve(file);
  const before = readFileSync(path, 'utf8');
  let after = before;
  for (const block of BLOCKS) after = replaceBlock(after, block, file);
  if (check) {
    if (after !== before) {
      stale++;
      console.log(`stale: ${file}`);
    } else {
      console.log(`ok: ${file}`);
    }
  } else if (after !== before) {
    writeFileSync(path, after);
    console.log(
      `inlined: ${file} (${(Buffer.byteLength(after) / 1024).toFixed(0)} KB)`,
    );
  } else {
    console.log(`unchanged: ${file}`);
  }
}
if (stale) process.exit(1);
