/** Durable policy checks for the desktop stylesheets. */

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { collectSourceFiles } from './lib/source';

const TOKENS = new URL('../kit/tokens.css', import.meta.url);
const STYLE_ENTRY = new URL('../src/styles/index.css', import.meta.url);
const SHELL = new URL('../src/styles/shell.css', import.meta.url);
const COMPONENTS = new URL('../src/styles/app.css', import.meta.url);
const FIRST_RUN = new URL('../src/screens/first-run.css', import.meta.url);
const SETTINGS = new URL('../src/screens/settings.css', import.meta.url);
const CASCADE = ['tokens', 'base', 'components', 'features', 'overrides'];
/**
 * Classes that only the first-run screens render and only the first-run sheet
 * selects, without the `local-` or `setup-` prefix the pattern below matches.
 */
const FIRST_RUN_ONLY_CLASSES = [
  'alt-path',
  'checked-address',
  'checklist',
  'checklist-actions',
  'checklist-cta',
  'checklist-notice',
  'choice-body',
  'facts',
  'foot-slot',
  'official-server',
  'operation-actions',
  'res',
  'server-address-row',
];
/**
 * Classes that only the settings pages render and only the settings sheet
 * selects. `settings-inset` is not listed: the group pages render it too.
 */
const SETTINGS_ONLY_CLASSES = [
  'account-fact-link',
  'account-main',
  'account-more',
  'device-list-actions',
  'devrow',
  'people-attention',
  'server-list-label',
  'settings-checkboxes',
  'settings-main',
  'settingslayout',
  'subnav-page',
  'subnav-side',
  'swatch',
  'swatches',
  'verified-mark',
];

/**
 * Every stylesheet under src/ and kit/ other than the token sheet and the
 * entry, found on disk so that a new sheet is held to the checks below
 * without being listed here.
 */
const STYLES = (
  await Promise.all(
    ['../src/', '../kit/'].map((directory) =>
      collectSourceFiles(new URL(directory, import.meta.url), ['.css']),
    ),
  )
)
  .flat()
  .filter(
    (sheet) => sheet.href !== TOKENS.href && sheet.href !== STYLE_ENTRY.href,
  )
  .sort((left, right) => left.href.localeCompare(right.href));
const TOKEN_ONLY_COLORS = STYLES;
const TYPE_FLOOR_PX = 12;

/** A stylesheet's path below apps/desktop/, for assertion messages. */
function label(sheet: URL): string {
  return sheet.pathname.split('/apps/desktop/')[1];
}

function blockAfter(source: string, marker: string): string {
  const markerIndex = source.indexOf(marker);
  assert.notEqual(markerIndex, -1, `missing ${marker}`);
  const open = source.indexOf('{', markerIndex + marker.length);
  assert.notEqual(open, -1, `missing block for ${marker}`);
  let depth = 1;
  let index = open + 1;
  while (index < source.length && depth > 0) {
    if (source[index] === '{') depth += 1;
    if (source[index] === '}') depth -= 1;
    index += 1;
  }
  assert.equal(depth, 0, `unterminated block for ${marker}`);
  return source.slice(open + 1, index - 1);
}

function declarations(block: string): Map<string, string> {
  const result = new Map<string, string>();
  const withoutComments = block.replace(/\/\*[\s\S]*?\*\//g, '');
  for (const declaration of withoutComments.split(';')) {
    const colon = declaration.indexOf(':');
    if (colon === -1 || declaration.includes('{')) continue;
    const property = declaration.slice(0, colon).trim();
    const value = declaration.slice(colon + 1).trim();
    if (property && value) result.set(property, value);
  }
  return result;
}

function rootTokens(css: string): Map<string, string> {
  const result = new Map<string, string>();
  for (const [property, value] of declarations(blockAfter(css, ':root'))) {
    if (property.startsWith('--')) result.set(property, value);
  }
  return result;
}

function typeSizes(css: string): { value: number; source: string }[] {
  const source = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const sizes: { value: number; source: string }[] = [];
  for (const match of source.matchAll(/font(?:-size)?\s*:\s*([^;}]+)/g)) {
    const value = match[1].trim();
    if (value === 'var(--mark-type)') continue;
    const pixels = /(\d+(?:\.\d+)?)px/.exec(value);
    if (pixels) sizes.push({ value: Number(pixels[1]), source: value });
  }
  return sizes;
}

test('the token sheet owns global custom properties', async () => {
  const kitTokens = rootTokens(await readFile(TOKENS, 'utf8'));

  assert.ok(kitTokens.size > 0, 'the shared token sheet is not empty');
  for (const sheet of STYLES) {
    const css = await readFile(sheet, 'utf8');
    assert.equal(
      /^\s*:root\s*\{/m.test(css),
      false,
      `${label(sheet)} declares global custom properties outside the token sheet`,
    );
    assert.equal(
      css.includes(':root[data-theme'),
      false,
      `${label(sheet)} contains theme decisions outside the token sheet`,
    );
  }
});

test('the stylesheet entry imports every stylesheet into a declared layer', async () => {
  const css = await readFile(STYLE_ENTRY, 'utf8');

  assert.ok(css.includes(`@layer ${CASCADE.join(', ')};`));
  assert.ok(STYLES.some((sheet) => sheet.href === SHELL.href));
  const imported: string[] = [];
  for (const line of css.split('\n')) {
    if (!line.startsWith('@import')) continue;
    const match = /^@import '([^']+)' layer\(([\w.-]+)\);$/.exec(line);
    assert.ok(match, `${line} does not import into a named layer`);
    const [, path, layer] = match;
    assert.ok(
      CASCADE.includes(layer.split('.')[0]),
      `${path} is imported into ${layer}, outside the declared cascade`,
    );
    imported.push(new URL(path, STYLE_ENTRY).href);
  }
  // A sheet missing here is never loaded; a sheet imported twice is a second
  // copy of its rules, possibly in a different layer.
  assert.deepEqual(
    [...imported].sort(),
    [TOKENS, ...STYLES].map((sheet) => sheet.href).sort(),
  );
  // An @import inside another sheet would load its target a second time,
  // outside the layer the entry assigns.
  for (const sheet of [TOKENS, ...STYLES])
    assert.equal(
      /^\s*@import\b/m.test(await readFile(sheet, 'utf8')),
      false,
      `${label(sheet)} imports a stylesheet; only the entry imports sheets`,
    );
});

/**
 * Fails if `sheet` no longer selects one of `classes`, so that the list cannot
 * go stale, or if app.css selects one, so that a rule for the feature is not
 * added back to the components layer.
 */
async function assertFeatureClasses(
  sheet: URL,
  classes: string[],
): Promise<void> {
  const components = (await readFile(COMPONENTS, 'utf8')).replace(
    /\/\*[\s\S]*?\*\//g,
    '',
  );
  const feature = await readFile(sheet, 'utf8');
  for (const name of classes) {
    const selector = new RegExp(`\\.${name}(?![\\w-])`);
    assert.ok(
      selector.test(feature),
      `${label(sheet)} no longer selects .${name}`,
    );
    assert.equal(
      selector.test(components),
      false,
      `app.css selects .${name}; the rule belongs in ${label(sheet)}`,
    );
  }
}

/** The `features.*` sublayers, in the order the entry declares them. */
async function featureLayers(): Promise<string[]> {
  const entry = await readFile(STYLE_ENTRY, 'utf8');
  return [...entry.matchAll(/layer\((features\.[\w-]+)\)/g)].map(
    (match) => match[1],
  );
}

test('first-run rules live in the first-run sheet, below the other features', async () => {
  const components = (await readFile(COMPONENTS, 'utf8')).replace(
    /\/\*[\s\S]*?\*\//g,
    '',
  );

  assert.equal(
    /\.(?:first-run-main|local-[\w-]+|setup-[\w-]+)\b/.test(components),
    false,
    'app.css holds a first-run rule; it belongs in screens/first-run.css',
  );
  await assertFeatureClasses(FIRST_RUN, FIRST_RUN_ONLY_CLASSES);
  // The first-run rules were in the components layer, under every feature's
  // rules; as the first feature sublayer they keep that order.
  assert.equal((await featureLayers())[0], 'features.first-run');
});

test('settings rules live in the settings sheet, in the sublayer after first-run', async () => {
  await assertFeatureClasses(SETTINGS, SETTINGS_ONLY_CLASSES);
  // The settings rules were in the components layer, under the files, chat,
  // teams and search rules; the sublayer after first-run keeps that order.
  assert.equal((await featureLayers())[1], 'features.settings');
});

test('only the entry point imports a stylesheet', async () => {
  // A stylesheet imported by a component sits outside the cascade layers and
  // outranks every layered rule; it also stops node from loading the module
  // without Vite.
  const importers: string[] = [];
  for (const directory of ['../src/', '../kit/'])
    for (const file of await collectSourceFiles(
      new URL(directory, import.meta.url),
    )) {
      const source = await readFile(file, 'utf8');
      if (/^\s*import\s+['"][^'"]+\.css['"]/m.test(source))
        importers.push(label(file));
    }
  assert.deepEqual(importers, ['src/main.tsx']);
});

test('authored text respects the 12px type floor', async () => {
  for (const sheet of STYLES) {
    const css = await readFile(sheet, 'utf8');
    for (const { value, source } of typeSizes(css)) {
      assert.ok(
        value >= TYPE_FLOOR_PX,
        `${label(sheet)} declares ${source}, below the ${TYPE_FLOOR_PX}px floor`,
      );
    }
    assert.equal(
      /font-size\s*:\s*[\d.]+(?:em|rem|pt|%)/.test(css),
      false,
      `${label(sheet)} uses a relative font size that bypasses the pixel floor`,
    );
  }
});

test('light and dark themes declare their native color scheme', async () => {
  const css = await readFile(TOKENS, 'utf8');
  const light = declarations(blockAfter(css, ':root'));
  const dark = declarations(blockAfter(css, ":root[data-theme='dark']"));

  assert.equal(light.get('color-scheme'), 'light');
  assert.equal(dark.get('color-scheme'), 'dark');
  for (const property of dark.keys()) {
    if (!property.startsWith('--')) continue;
    assert.ok(light.has(property), `${property} has no light-theme default`);
  }
});

test('feature styles use design tokens for colors', async () => {
  for (const sheet of TOKEN_ONLY_COLORS) {
    const css = (await readFile(sheet, 'utf8')).replace(
      /\/\*[\s\S]*?\*\//g,
      '',
    );
    assert.equal(
      /#[0-9a-f]{3,8}\b|\brgba?\(/i.test(css),
      false,
      `${label(sheet)} contains a literal color; add a semantic token instead`,
    );
  }
});

test('reduced motion is enforced across every component stylesheet', async () => {
  // Whitespace is collapsed so that the selector list matches however the
  // formatter wraps it; the sheet has several reduced-motion blocks.
  const css = (await readFile(SHELL, 'utf8')).replace(/\s+/g, ' ');
  const marker = '@media (prefers-reduced-motion: reduce)';
  const universal = '*, *::before, *::after';
  const policies: string[] = [];
  for (
    let at = css.indexOf(marker);
    at !== -1;
    at = css.indexOf(marker, at + 1)
  )
    policies.push(blockAfter(css.slice(at), marker));
  const policy = policies.find((block) => block.includes(universal));
  assert.ok(policy, 'no reduced-motion block resets every element');
  const values = declarations(blockAfter(policy, universal));

  assert.equal(values.get('scroll-behavior'), 'auto !important');
  assert.equal(values.get('animation-duration'), '0.01ms !important');
  assert.equal(values.get('animation-iteration-count'), '1 !important');
  assert.equal(values.get('transition-duration'), '0.01ms !important');
});
