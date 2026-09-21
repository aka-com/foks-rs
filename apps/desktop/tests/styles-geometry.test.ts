/**
 * Static tests verifying stylesheet invariants, token declarations, and layout rules.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { readSource } from './lib/source';

const SHELL = '../src/styles/shell.css';
const TOKENS = '../kit/tokens.css';
/** The stylesheets that carry every authored type size in the desktop app. */
const SHEETS = [SHELL, '../src/styles/app.css', '../src/screens/chat.css'];
/** Nothing in the app draws text below this size. */
const TYPE_FLOOR_PX = 12;

/** The `:root` declarations in a stylesheet, as name → value. */
function rootTokens(css: string): Map<string, string> {
  const open = /:root\s*\{/.exec(css);
  assert.ok(open, 'the stylesheet declares :root');
  let index = open.index + open[0].length;
  let depth = 1;
  const start = index;
  while (index < css.length && depth > 0) {
    if (css[index] === '{') depth += 1;
    else if (css[index] === '}') depth -= 1;
    index += 1;
  }
  const block = css.slice(start, index - 1).replace(/\/\*[\s\S]*?\*\//g, '');
  const tokens = new Map<string, string>();
  for (const declaration of block.split(';')) {
    const match = /^\s*(--[\w-]+)\s*:\s*([\s\S]+?)\s*$/.exec(declaration);
    if (match) tokens.set(match[1], match[2].replace(/\s+/g, ' ').trim());
  }
  return tokens;
}

test('the shared tokens come from the kit and are not re-declared here', async () => {
  const shell = await readSource(SHELL, import.meta.url);
  const kit = await readSource(TOKENS, import.meta.url);

  assert.match(shell, /@import '\/kit\/tokens\.css';/);
  // The kit import must precede local declarations so local overrides take precedence.
  assert.ok(
    shell.indexOf("@import '/kit/tokens.css';") < shell.indexOf(':root{'),
    'the kit import comes before the local :root block',
  );

  const kitTokens = rootTokens(kit);
  const shellTokens = rootTokens(shell);
  assert.equal(kitTokens.size, 43, 'expected exactly 43 shared kit tokens');

  for (const name of shellTokens.keys()) {
    assert.equal(
      kitTokens.has(name),
      false,
      `${name} is declared in both apps/desktop/kit/tokens.css and the FOKS shell`,
    );
  }
});

test('declares the 18 expected FOKS design tokens', async () => {
  const shell = await readSource(SHELL, import.meta.url);
  const tokens = rootTokens(shell);

  assert.deepEqual([...tokens.keys()].sort(), [
    '--c-file',
    '--c-link',
    '--c-none',
    '--c-password',
    '--c-resource',
    '--c-team',
    '--faint',
    '--hover',
    '--main-surface',
    '--mono',
    '--radius-lg',
    '--radius-md',
    '--radius-pill',
    '--radius-sm',
    '--radius-xl',
    '--sans',
    '--shadow-menu',
    '--surface',
  ]);

  // Verify tokens with application-specific overrides retain local values.
  assert.equal(tokens.get('--faint'), '#8a8a92');
  assert.equal(tokens.get('--surface'), '#f6f6f9');
  assert.equal(tokens.get('--main-surface'), '#fff');
  assert.equal(tokens.get('--hover'), 'rgba(0,0,0,.05)');
  assert.equal(tokens.get('--shadow-menu'), '0 10px 28px rgba(15,20,45,.14)');
});

/**
 * Every authored type size in a stylesheet, as `{ value, source }` pairs. Both
 * the longhand (`font-size:11px`) and the size component of the `font:`
 * shorthand (`font:600 12px/16px inherit`) are collected; the shorthand's size
 * is its first px length, because the weight and style keywords that may
 * precede it carry no unit and the line-height that may follow it is separated
 * by a slash.
 */
function typeSizes(css: string): { value: number; source: string }[] {
  const declarations = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const sizes: { value: number; source: string }[] = [];
  for (const match of declarations.matchAll(/font(-size)?\s*:\s*([^;}]+)/g)) {
    const value = match[2].trim();
    // The one exemption: a store mark's initials, sized by `--mark-type` in
    // tokens.css. They are a glyph in a tile, not text a reader runs through.
    if (value === 'var(--mark-type)' || /^--mark-type/.test(match[0])) continue;
    const px = /(\d+(?:\.\d+)?)px/.exec(value);
    if (px) sizes.push({ value: Number(px[1]), source: `font: ${value}` });
  }
  return sizes;
}

test('no stylesheet draws text below the 12px type floor', async () => {
  for (const sheet of SHEETS) {
    const css = await readSource(sheet, import.meta.url);
    for (const { value, source } of typeSizes(css)) {
      assert.ok(
        value >= TYPE_FLOOR_PX,
        `${sheet} declares ${value}px, below the ${TYPE_FLOOR_PX}px floor: ${source}`,
      );
    }
    // Relative type sizes would escape the scan above, so none are allowed.
    assert.doesNotMatch(
      css.replace(/\/\*[\s\S]*?\*\//g, ''),
      /font-size\s*:\s*[\d.]+(em|rem|pt|%)/,
      `${sheet} sizes text in a relative unit, which the floor cannot check`,
    );
  }
  // `<small>` would otherwise fall to the UA's 0.8333em inside a 13px row.
  const shell = await readSource(SHELL, import.meta.url);
  assert.match(shell, /(^|})small\{font-size:12px\}/m);
});

test('stylesheet contains no dark mode media queries or theme overrides', async () => {
  const strip = (css: string): string => css.replace(/\/\*[\s\S]*?\*\//g, '');
  const shell = strip(await readSource(SHELL, import.meta.url));
  const tokens = strip(await readSource(TOKENS, import.meta.url));
  for (const css of [shell, tokens]) {
    assert.doesNotMatch(css, /prefers-color-scheme/);
    assert.doesNotMatch(css, /\[data-theme/);
  }
});

test('shell stylesheet contains required grid and flexbox layout rules', async () => {
  const shell = await readSource(SHELL, import.meta.url);

  // The two- and three-column shells (sidebar · items · details). The track
  // widths are named by the layout tokens declared on `.app`.
  assert.match(
    shell,
    /\.app\{[^}]*grid-template-columns:var\(--side-track\) minmax\(0,1fr\)[;}]/,
  );
  assert.match(
    shell,
    /\.app\.with-details\{grid-template-columns:var\(--side-track\) minmax\(0,1fr\) var\(--details-w\)\}/,
  );
  // Layout tokens live on the shell, never on `:root`. The rail width is on
  // `.window` rather than `.app` because the takeovers that leave the rail
  // visible are siblings of `.app`, outside the background a dialog makes
  // inert.
  assert.match(shell, /\.window\{[^}]*--topbar-h:44px[;}]/);
  assert.match(shell, /\.window\{[^}]*--side-w-open:224px[;}]/);
  assert.match(shell, /\.app\{[^}]*--details-w:300px[;}]/);
  assert.match(shell, /\.window:has\(>\.app\.side-narrow\)\{--side-w:46px\}/);
  assert.match(
    shell,
    /\.takeover\{position:absolute;inset:var\(--topbar-h\) 0 0 var\(--side-w\);z-index:18;display:grid;place-items:center;padding:20px;overflow:auto\}/,
  );
  assert.doesNotMatch(shell, /\.stop(?:wrap|veil)\{[^}]*inset:/);
  assert.match(
    shell,
    /\.window:has\(>\.app>\.first-run-main\)>\.takeover\{top:0\}/,
  );
  // The collapsed rail is a fixed track: no hover or focus expansion.
  assert.match(shell, /\.side\.is-narrow[^{]*\{/);
  assert.doesNotMatch(shell, /\.side\.is-narrow[^{]*:hover/);
  assert.doesNotMatch(shell, /is-pinned/);
  // The topbar carries the shell's own controls and the only hairline above
  // the page; the page header below it carries none.
  assert.match(
    shell,
    /\.topbar\{height:var\(--topbar-h\);[^}]*border-bottom:1px solid var\(--line-soft\)/,
  );
  assert.match(shell, /\.topbar \.topsearch\{[^}]*width:240px/);
  // The vault's scoped search is the same width as the topbar's own field,
  // and the kind filter's options are each as wide as their own label.
  assert.match(shell, /\.toolbar-rest \.search\{width:240px/);
  assert.match(shell, /\.toolbar-rest>\.search\{margin-left:auto\}/);
  assert.match(shell, /\.toolbar-filter \.seg button,[^{]*\{flex:0 1 auto/);
  // Allow main column flex shrinking to prevent horizontal window overflow.
  assert.match(shell, /\.main\{[^}]*min-width:0[^}]*\}/);
  assert.match(shell, /\.path\{[^}]*padding:14px 20px 10px;/);
  assert.match(shell, /\.header-action\{[^}]*margin-left:auto;[^}]*flex:none/);
  assert.match(
    shell,
    /\.settings-main>\.settings-inset\+\.band\{margin-top:6px\}/,
  );
  // Title row flex shrinkage preserves action button visibility in headers.
  assert.match(shell, /\.loc\{[^}]*flex:1 1 auto/);
  assert.match(shell, /\.loc-copy\{[^}]*white-space:nowrap/);
  assert.match(shell, /\.header-action \.btn\.cap\{height:28px;font-size:13px/);
  assert.match(
    shell,
    /\.loc h1\{display:inline;font-size:20px;font-weight:700/,
  );
  assert.match(shell, /\.loc small\{display:inline;/);
  // Ensure CSS row height matches the virtual list row estimate.
  assert.match(shell, /\.row\{height:50px;/);
  assert.match(shell, /\.toolbar \.btn,\.toolbar \.seg\{height:32px\}/);
  assert.match(
    shell,
    /\.toolbar \.btn\.primary,\.empty \.btn\.primary\{height:30px\}/,
  );
  assert.match(
    shell,
    /\.toolbar \.btn,\.toolbar \.seg\.txt button\{font-size:13px\}/,
  );
  assert.match(shell, /\.btn \.ic\.chevron\{font-size:14px\}/);
  assert.match(shell, /\.menu button\{[^}]*padding:6px 9px;/);
  assert.match(shell, /\.menu button\.kind-menu-item\{padding-block:4px\}/);
  // Maintain accent background color on hover for disabled primary buttons.
  assert.match(
    shell,
    /\.btn\.primary\[disabled\]:hover\{background:var\(--accent\)\}/,
  );
  assert.match(
    shell,
    /\.meta code\{[^}]*overflow-wrap:anywhere[^}]*word-break:normal/,
  );
  assert.match(
    shell,
    /\.folder-split\{[^}]*grid-template-columns:236px minmax\(0,1fr\)/,
  );
  assert.match(
    shell,
    /\.tpane\{[^}]*background:var\(--main-surface\)[^}]*padding:6px 6px 8px/,
  );
  assert.match(shell, /\.tpane \.fn\{[^}]*font-weight:500/);
  assert.doesNotMatch(shell, /\.tpane \.fn\.on\{[^}]*font-weight/);
  // Tree rows indent by depth; a root icon sits on the 16px edge the crumb
  // and headings share, and a nested folder with children gives one indent
  // back to its twist so its icon lands where a childless sibling's does.
  assert.match(
    shell,
    /\.tpane \.fn\{[^}]*padding-left:calc\(4px \+ var\(--d,0\) \* 18px\)/,
  );
  assert.match(shell, /\.tpane \.fn \.fselect\{[^}]*padding:0 8px 0 6px/);
  assert.match(
    shell,
    /\.tpane \.fn:has\(>\.twist\)\{padding-left:calc\(4px \+ var\(--d,0\) \* 18px - 18px\)\}/,
  );
  assert.doesNotMatch(shell, /\.fselect\{padding-left:24px\}/);
  assert.match(shell, /\.tpane h6\{[^}]*padding:4px 10px/);
  assert.match(shell, /\.twist\{[^}]*flex:none;width:18px;height:18px/);
  assert.match(shell, /\.radio\{[^}]*text-align:left[^}]*width:100%/);
  // Layout server settings rows with right-aligned action banners.
  assert.match(shell, /\.settings-inset \.fr \.v\.srv\{flex-direction:row/);
  assert.match(shell, /\.band \.a\{margin-left:auto/);
  // Prevent chip elements from stretching full width inside flex value columns.
  assert.match(shell, /\.chip\{[^}]*width:max-content/);
  assert.match(
    shell,
    /\.settings-inset \.fr \.v \.chip\{align-self:flex-start\}/,
  );
  // Ensure form inputs expand to fill the field row value column.
  assert.match(shell, /\.inset \.fr \.v\{flex:1;min-width:0/);
  assert.match(shell, /\.inset \.fr input\{[^}]*width:100%/);
  // Toast notifications must render with higher z-index than modal backdrops.
  assert.match(shell, /\.backdrop\{[^}]*z-index:10\}/);
  assert.match(shell, /\.toasts\{[^}]*position:fixed;[^}]*z-index:20/);
});

test('disabled detail actions stay visible without active hover or pointer styling', async () => {
  const shell = await readSource(SHELL, import.meta.url);
  assert.match(
    shell,
    /\.details \.prev \.irow \.a button\[disabled\],\.details \.inset\.edit \.fr \.a button\[disabled\]\{opacity:\.5;cursor:default\}/,
  );
  assert.match(shell, /\.prev \.irow \.a button:hover:not\(:disabled\)/);
  assert.match(
    shell,
    /\.details \.inset\.edit \.fr \.a button:hover:not\(:disabled\)/,
  );
});

test('app stylesheet uses design tokens and declares no hardcoded colors', async () => {
  const app = await readSource('../src/styles/app.css', import.meta.url);
  // Enforce CSS variables for all color values to prevent design drift.
  const declarations = app.replace(/\/\*[\s\S]*?\*\//g, '');
  assert.doesNotMatch(declarations, /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(declarations, /\brgba?\(/);
  assert.match(app, /background: var\(--main-surface\)/);
  assert.match(
    app,
    /\.first-run-main \.checklist \.fr \.v\s*\{[^}]*display: flex;[^}]*flex-direction: column;/,
  );
  assert.match(app, /\.first-run-main \.crit\s*\{[^}]*margin-bottom: 16px;/);
  assert.doesNotMatch(app, /(?:^|\n)\.crit\s*\{/);
  assert.match(
    app,
    /\.local-field-row\s*\{[^}]*min-height: 36px;[^}]*padding: 0 16px;/,
  );
  // The account pages' numbered section labels: the numeral is an 18px disc
  // drawn from the chip tokens, and the phrase rows joined the account form,
  // so no separate `.recovery-fields` geometry survives.
  assert.match(
    app,
    /\.first-run-main \.sec\.step \.n\s*\{[^}]*width: 18px;[^}]*height: 18px;[^}]*border-radius: 50%;[^}]*background: var\(--chip-bg\);/,
  );
  assert.doesNotMatch(app, /\.recovery-fields|\.signin-methods/);
  assert.match(
    app,
    /\.first-run-main \.account-form \.fr\s*\{[^}]*align-items: stretch;[^}]*min-height: 36px;[^}]*padding: 0;/,
  );
  assert.match(
    app,
    /\.first-run-main \.account-form \.fr \.k\s*\{[^}]*display: flex;[^}]*align-items: center;[^}]*width: 140px;[^}]*white-space: nowrap;/,
  );
  assert.match(
    app,
    /\.first-run-main \.account-form \.fr \.v\s*\{[^}]*display: flex;[^}]*flex: 1;/,
  );
  assert.match(
    app,
    /\.first-run-main \.account-form \.fr input\s*\{[^}]*align-self: stretch;[^}]*width: 100%;[^}]*padding: 0 14px;/,
  );
  // A roster table is a list of flex rows, not a grid with a header row: no
  // column template, and no `.hdr` rule, survives for `.rt`.
  assert.match(app, /\.rt\.bare \.prow\s*\{[^}]*display: flex;/);
  assert.doesNotMatch(app, /\.rt[^{,]*\.hdr/);
  assert.doesNotMatch(app, /\.rt[^{]*\{[^}]*grid-template-columns/);
  assert.match(app, /\.tab\.on::after/);
  assert.match(app, /\.inset\.danger\s*\{[^}]*overflow: visible/);
  assert.match(
    app,
    /\.verified-mark \.ic\s*\{[^}]*transform: translate\(-1px, 1px\);/,
  );
});

test('refresh status uses a fixed overlay and an opaque bounded surface', async () => {
  const app = await readSource('../src/styles/app.css', import.meta.url);
  const shell = await readSource(SHELL, import.meta.url);
  assert.match(app, /\.menu-portal\s*\{[^}]*position: fixed;/);
  assert.match(app, /\.menu-portal\s*\{[^}]*z-index: 16;/);
  const popover = /\.sync-popover\{([^}]*)\}/.exec(shell)?.[1] ?? '';
  assert.match(popover, /background:var\(--main-surface\)/);
  assert.match(popover, /border:1px solid var\(--line\)/);
  assert.match(popover, /box-shadow:var\(--shadow-menu\)/);
  assert.match(popover, /max-height:calc\(100vh - 16px\)/);
  assert.match(popover, /overflow:auto/);
});

test('the rail reserves the strip macOS draws its window controls on', async () => {
  const app = await readSource('../src/styles/app.css', import.meta.url);
  // There is no title bar: the rail's own drag strip is where the controls go.
  const shell = await readSource(SHELL, import.meta.url);
  assert.doesNotMatch(shell, /\.titlebar/);
  assert.match(
    app,
    /#root > \.native-window \.traffic\s*\{[^}]*min-height: 38px;/,
  );
  assert.match(
    app,
    /#root > \.native-window\.window-chrome-hidden \.traffic\s*\{[^}]*min-height: 32px;/,
  );
});
