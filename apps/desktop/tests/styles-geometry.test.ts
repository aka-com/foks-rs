/** Durable policy checks for the desktop stylesheets. */

import assert from 'node:assert/strict';
import test from 'node:test';

import { readSource } from './lib/source';

const TOKENS = '../kit/tokens.css';
const STYLE_ENTRY = '../src/styles/index.css';
const SHELL = '../src/styles/shell.css';
const STYLES = [
  SHELL,
  '../src/styles/app.css',
  '../src/styles/native-window.css',
  '../src/screens/files.css',
  '../src/screens/chat.css',
  '../src/screens/chat-picker.css',
  '../src/screens/teams.css',
  '../src/screens/devices.css',
  '../src/screens/settings.css',
  '../src/screens/first-run.css',
  '../src/shell/search-palette.css',
];
const TOKEN_ONLY_COLORS = STYLES;
const TYPE_FLOOR_PX = 12;

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
  const kitTokens = rootTokens(await readSource(TOKENS, import.meta.url));

  assert.ok(kitTokens.size > 0, 'the shared token sheet is not empty');
  for (const sheet of STYLES) {
    const css = await readSource(sheet, import.meta.url);
    assert.equal(
      /^\s*:root\s*\{/m.test(css),
      false,
      `${sheet} declares global custom properties outside the token sheet`,
    );
    assert.equal(
      css.includes(':root[data-theme'),
      false,
      `${sheet} contains theme decisions outside the token sheet`,
    );
  }
});

test('the stylesheet entry declares the cascade and imports each style area', async () => {
  const css = await readSource(STYLE_ENTRY, import.meta.url);

  assert.ok(
    css.includes('@layer tokens, base, components, features, overrides'),
  );
  for (const sheet of [TOKENS, ...STYLES]) {
    const filename = sheet.split('/').at(-1);
    assert.ok(
      filename && css.includes(filename),
      `${sheet} is outside the cascade`,
    );
  }
});

test('authored text respects the 12px type floor', async () => {
  for (const sheet of STYLES) {
    const css = await readSource(sheet, import.meta.url);
    for (const { value, source } of typeSizes(css)) {
      assert.ok(
        value >= TYPE_FLOOR_PX,
        `${sheet} declares ${source}, below the ${TYPE_FLOOR_PX}px floor`,
      );
    }
    assert.equal(
      /font-size\s*:\s*[\d.]+(?:em|rem|pt|%)/.test(css),
      false,
      `${sheet} uses a relative font size that bypasses the pixel floor`,
    );
  }
});

test('light and dark themes declare their native color scheme', async () => {
  const css = await readSource(TOKENS, import.meta.url);
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
    const css = (await readSource(sheet, import.meta.url)).replace(
      /\/\*[\s\S]*?\*\//g,
      '',
    );
    assert.equal(
      /#[0-9a-f]{3,8}\b|\brgba?\(/i.test(css),
      false,
      `${sheet} contains a literal color; add a semantic token instead`,
    );
  }
});

test('reduced motion trades travel for fades across every component stylesheet', async () => {
  const css = await readSource(SHELL, import.meta.url);
  const policy = blockAfter(css, '@media (prefers-reduced-motion: reduce)');
  const values = declarations(blockAfter(policy, '*, *::before, *::after'));

  assert.equal(values.get('scroll-behavior'), 'auto !important');
  assert.equal(values.get('animation-iteration-count'), '1 !important');
  // Transitions keep colour and opacity, so state changes still fade, but
  // nothing that moves (transform, position, size) is animated.
  const properties = (values.get('transition-property') ?? '')
    .replace('!important', '')
    .split(',')
    .map((property) => property.trim());
  assert.ok(properties.includes('opacity'));
  for (const moving of ['transform', 'left', 'top', 'width', 'height', 'all'])
    assert.equal(properties.includes(moving), false, `${moving} still moves`);
  // The overlay keyframes read their travel from tokens that reduced motion
  // zeroes, so sheets, alerts and toasts play as plain fades.
  const tokens = await readSource(TOKENS, import.meta.url);
  const reduced = declarations(
    blockAfter(
      blockAfter(tokens, '@media (prefers-reduced-motion: reduce)'),
      ':root',
    ),
  );
  assert.equal(reduced.get('--rise'), '0px');
  assert.equal(reduced.get('--settle-scale'), '1');
});

test('a first-run step taller than the window scrolls its body and keeps its foot in view', async () => {
  const firstRun = await readSource(
    '../src/screens/first-run.css',
    import.meta.url,
  );
  const shell = await readSource(SHELL, import.meta.url);
  // The card is no taller than the stage, and its one row is no taller than
  // the card: without the row limit the row grows to the step's content and
  // the card's overflow clip cuts the foot off at 640px.
  const card = declarations(blockAfter(firstRun, '\n.fr-card '));
  assert.equal(card.get('max-height'), '100%');
  assert.equal(card.get('grid-template-rows'), 'minmax(0, 1fr)');
  const column = declarations(
    blockAfter(firstRun, '.fr-card > .main.first-run-main'),
  );
  assert.equal(column.get('min-height'), '0');
  // The step column is the shell's flex column: `.body` scrolls and the
  // `.pfoot` after it does not shrink.
  assert.match(shell, /\.main\{[^}]*min-height:0;[^}]*flex-direction:column/);
  assert.match(shell, /\.body\{flex:1;min-height:0;overflow:auto/);
  assert.match(shell, /\.pfoot\{flex:none/);
});
