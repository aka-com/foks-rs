/**
 * What the stylesheet must keep being, checked against its source.
 *
 * jsdom computes no layout, so the geometry the design depends on cannot be
 * asserted by rendering. These read `src/styles/shell.css` — which is
 * `wave6/shell.css` lifted whole — and hold the few structural facts that a
 * careless edit would quietly break: the token split, the sidebar's width,
 * and the fact that nothing here re-declares a token the kit already owns.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { readSource } from './lib/source';

const SHELL = '../src/styles/shell.css';
const TOKENS = '../../ui/kit/tokens.css';

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
  // The import must precede the local block, or the cascade hands FOKS AKA's
  // values for the five that diverge.
  assert.ok(
    shell.indexOf("@import '/kit/tokens.css';") < shell.indexOf(':root{'),
    'the kit import comes before the local :root block',
  );

  const kitTokens = rootTokens(kit);
  const shellTokens = rootTokens(shell);
  assert.equal(kitTokens.size, 35, 'the measured shared subset');

  for (const name of shellTokens.keys()) {
    assert.equal(
      kitTokens.has(name),
      false,
      `${name} is declared in both ui/kit/tokens.css and the FOKS shell`,
    );
  }
});

test('the nineteen FOKS tokens are the five that diverge and the fourteen that are ours', async () => {
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
    '--ok-wash',
    '--radius-lg',
    '--radius-md',
    '--radius-pill',
    '--radius-sm',
    '--radius-xl',
    '--sans',
    '--shadow-menu',
    '--surface',
  ]);

  // The five that share a name with AKA keep the design's values, not AKA's.
  assert.equal(tokens.get('--faint'), '#8a8a92');
  assert.equal(tokens.get('--surface'), '#f6f6f9');
  assert.equal(tokens.get('--main-surface'), '#fff');
  assert.equal(tokens.get('--hover'), 'rgba(0,0,0,.05)');
  assert.equal(tokens.get('--shadow-menu'), '0 10px 28px rgba(15,20,45,.14)');
});

test('the sheet is light only — no dark override travelled with it', async () => {
  const strip = (css: string): string => css.replace(/\/\*[\s\S]*?\*\//g, '');
  const shell = strip(await readSource(SHELL, import.meta.url));
  const tokens = strip(await readSource(TOKENS, import.meta.url));
  for (const css of [shell, tokens]) {
    assert.doesNotMatch(css, /prefers-color-scheme/);
    assert.doesNotMatch(css, /\[data-theme/);
  }
});

test('the shell layout the design specifies survived the lift', async () => {
  const shell = await readSource(SHELL, import.meta.url);

  // The two- and three-column shells (sidebar · items · details).
  assert.match(shell, /\.app\{[^}]*grid-template-columns:224px 1fr\}/);
  assert.match(
    shell,
    /\.app\.with-details\{grid-template-columns:224px 1fr 300px\}/,
  );
  // Group roster stacks sit on the trailing edge, including a one-person
  // stack that would otherwise rest on the left of the 28px slot.
  assert.match(shell, /\.nav \.stack\{margin-left:auto\}/);
  assert.match(shell, /\.nav \.stack \.av:only-child\{right:0\}/);
  // The main column must be allowed to shrink, or a long path scrolls the
  // window instead of the list.
  assert.match(shell, /\.main\{[^}]*min-width:0[^}]*\}/);
  assert.match(
    shell,
    /\.path\{[^}]*padding:8px 20px;[^}]*border-bottom:1px solid var\(--line-soft\)/,
  );
  assert.match(shell, /\.header-action\{[^}]*margin-left:auto;[^}]*flex:none/);
  // The title row must shrink so takeover actions stay in `.header-action`
  // instead of being pushed off the clipped window by a long group subtitle.
  assert.match(shell, /\.loc\{[^}]*flex:1 1 auto/);
  assert.match(shell, /\.loc-copy\{[^}]*white-space:nowrap/);
  assert.match(shell, /\.header-action \.btn\.cap\{height:28px;font-size:13px/);
  assert.match(shell, /\.loc h1\{display:inline;/);
  assert.match(shell, /\.loc small\{display:inline;/);
  // The windowing estimate in ItemsScreen and the design's fixed row must agree.
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
  // Disabled primary hover must not fall back to --btn-bg, or the blue
  // fill disappears against the card while the pointer is still over it.
  assert.match(
    shell,
    /\.btn\.primary\[disabled\]:hover\{background:var\(--accent\)\}/,
  );
  assert.match(
    shell,
    /\.meta code\{[^}]*overflow-wrap:anywhere[^}]*word-break:normal/,
  );
  assert.match(shell, /\.tile \.qa\{[^}]*right:8px;top:8px;/);
  assert.match(shell, /\.radio\{[^}]*text-align:left[^}]*width:100%/);
  // The servers rows sit in a settings inset: the value column runs across,
  // and the band's action hugs its right edge.
  assert.match(shell, /\.settings-inset \.fr \.v\.srv\{flex-direction:row/);
  assert.match(shell, /\.band \.a\{margin-left:auto/);
  // Settings value columns are a flex stack; chips must hug their text
  // instead of stretching across the row.
  assert.match(shell, /\.chip\{[^}]*width:max-content/);
  assert.match(
    shell,
    /\.settings-inset \.fr \.v \.chip\{align-self:flex-start\}/,
  );
  // Sheet field rows: the value column grows, and the input fills it, so a
  // click anywhere in the row hits the control rather than a shrink-wrapped
  // text width.
  assert.match(shell, /\.inset \.fr \.v\{flex:1;min-width:0/);
  assert.match(shell, /\.inset \.fr input\{[^}]*width:100%/);
  // Toasts share `#overlays` with sheets; they must paint above `.backdrop`.
  assert.match(shell, /\.backdrop\{[^}]*z-index:10\}/);
  assert.match(shell, /\.toasts\{[^}]*position:fixed;[^}]*z-index:20/);
});

test('the app sheet only adapts the mock window; it invents no colours', async () => {
  const app = await readSource('../src/styles/app.css', import.meta.url);
  // Every colour must be a token: a literal here is a place the design and
  // the app can drift without the design being able to see it.
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
  assert.match(
    app,
    /\.first-run-main \.recovery-fields \.fr\s*\{[^}]*align-items: stretch;[^}]*min-height: 36px;[^}]*padding: 0;/,
  );
  assert.match(
    app,
    /\.first-run-main \.recovery-fields \.fr \.k\s*\{[^}]*display: flex;[^}]*align-items: center;[^}]*width: 140px;[^}]*white-space: nowrap;/,
  );
  assert.match(
    app,
    /\.first-run-main \.recovery-fields \.fr \.v\s*\{[^}]*display: flex;[^}]*flex: 1;/,
  );
  assert.match(
    app,
    /\.first-run-main \.recovery-fields \.fr input\s*\{[^}]*align-self: stretch;[^}]*width: 100%;[^}]*padding: 0 14px;/,
  );
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
  assert.match(app, /\.app-lock-card p\s*\{[^}]*margin-bottom: 20px;/);
  assert.match(
    app,
    /\.rt \.hdr,\s*\.rt \.prow\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) 168px 36px/,
  );
  assert.match(
    app,
    /\.rt\.fed \.hdr,\s*\.rt\.fed \.prow\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) 150px 168px 80px 150px/,
  );
  assert.match(
    app,
    /\.rt\.items \.hdr,\s*\.rt\.items \.prow\s*\{[^}]*grid-template-columns: minmax\(0, 1fr\) 118px 118px 96px 64px/,
  );
  assert.match(app, /\.tab\.on::after/);
  assert.match(app, /\.inset\.danger\s*\{[^}]*overflow: visible/);
});

test('the desktop title clears the macOS traffic lights', async () => {
  const app = await readSource('../src/styles/app.css', import.meta.url);
  assert.match(
    app,
    /#root > \.native-window > \.titlebar\s*\{[^}]*padding-left: 5\.5rem;/,
  );
  assert.match(
    app,
    /#root > \.window > \.titlebar \.brand\s*\{[^}]*transform: translateY\(1px\);/,
  );
  assert.match(
    app,
    /#root > \.window > \.titlebar > \.agent\s*\{[^}]*margin-right: 0\.25rem;[^}]*transform: translateY\(1px\);/,
  );
});
