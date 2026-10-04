#!/usr/bin/env node
// Icon source for the mockup kit.
//
// Reads Lucide icon nodes from the repository's node_modules (the package the
// desktop app renders its icons from) and emits them as inner-SVG strings.
//
//   node icons.mjs --kit             rewrite the ICONS block in kit.js
//   node icons.mjs name [name ...]   print entries for Kit.registerIcons()
//
// Names are Lucide's kebab-case names; aliases resolve (trash-2, circle-help).

import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const here = new URL('.', import.meta.url);
const lucide = await import(
  new URL('../../../../node_modules/lucide/dist/esm/lucide.mjs', here).href
);

// The kit's built-in set. Keep sorted; `node icons.mjs --kit` after editing.
export const KIT_ICONS = [
  'archive',
  'arrow-down',
  'arrow-left',
  'arrow-up',
  'arrow-up-down',
  'at-sign',
  'bell',
  'bell-off',
  'bold',
  'bot',
  'check',
  'check-check',
  'chevron-down',
  'chevron-left',
  'chevron-right',
  'chevron-up',
  'circle-alert',
  'circle-check',
  'circle-help',
  'clock',
  'code',
  'copy',
  'dices',
  'door-open',
  'download',
  'ellipsis',
  'external-link',
  'eye',
  'eye-off',
  'file',
  'file-text',
  'fingerprint',
  'flag',
  'folder',
  'folder-input',
  'folder-open',
  'globe',
  'hash',
  'history',
  'inbox',
  'info',
  'italic',
  'key',
  'key-round',
  'keyboard',
  'laptop',
  'layout-grid',
  'link',
  'list',
  'list-filter',
  'loader-circle',
  'lock',
  'log-out',
  'mail',
  'message-square',
  'monitor',
  'moon',
  'panel-left',
  'panel-left-close',
  'panel-left-open',
  'paperclip',
  'paw-print',
  'pencil',
  'pin',
  'plug',
  'plus',
  'quote',
  'refresh-cw',
  'reply',
  'search',
  'send',
  'server',
  'settings',
  'shield',
  'shield-alert',
  'shield-check',
  'smartphone',
  'smile',
  'square-check',
  'star',
  'sun',
  'terminal',
  'trash',
  'trash-2',
  'triangle-alert',
  'undo-2',
  'unlock',
  'upload',
  'user',
  'user-plus',
  'users',
  'vault',
  'wand-sparkles',
  'x',
];

const pascal = (name) =>
  name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');

const escAttr = (value) =>
  String(value)
    .replace(/&/g, '&amp;')
    .replace(/"/g, '&quot;')
    .replace(/</g, '&lt;');

export function inner(name) {
  const node = lucide[pascal(name)];
  if (!Array.isArray(node)) throw new Error(`Unknown Lucide icon: ${name}`);
  return node
    .map(([tag, attrs]) => {
      const list = Object.entries(attrs)
        .filter(([key]) => key !== 'key')
        .map(([key, value]) => `${key}="${escAttr(value)}"`)
        .join(' ');
      return `<${tag} ${list}/>`;
    })
    .join('');
}

function entries(names) {
  return names.map((name) => `  '${name}': '${inner(name)}',`).join('\n');
}

const args = process.argv.slice(2);
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (args[0] === '--kit') {
    const target = new URL('kit.js', here);
    const source = readFileSync(target, 'utf8');
    const begin = '/* ICONS:BEGIN */';
    const end = '/* ICONS:END */';
    const a = source.indexOf(begin);
    const b = source.indexOf(end);
    if (a < 0 || b < a) throw new Error('kit.js has no ICONS:BEGIN/END block');
    const block = `${begin}\n${entries(KIT_ICONS)}\n  ${end}`;
    writeFileSync(
      target,
      source.slice(0, a) + block + source.slice(b + end.length),
    );
    console.log(`kit.js: ${KIT_ICONS.length} icons`);
  } else if (args.length) {
    console.log(`Kit.registerIcons({\n${entries(args)}\n});`);
  } else {
    console.log(
      'usage: node icons.mjs --kit | node icons.mjs <lucide-name> ...',
    );
  }
}
