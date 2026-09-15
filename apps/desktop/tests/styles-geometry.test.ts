/**
 * Static tests verifying stylesheet invariants, token declarations, and layout rules.
 */

import assert from 'node:assert/strict';
import test from 'node:test';

import { readSource } from './lib/source';

const SHELL = '../src/styles/shell.css';
const TOKENS = '../kit/tokens.css';

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
  assert.equal(kitTokens.size, 37, 'expected exactly 37 shared kit tokens');

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
  // Layout tokens live on `.app`, never on `:root`.
  assert.match(shell, /\.app\{[^}]*--side-w-open:224px[;}]/);
  assert.match(shell, /\.app\{[^}]*--details-w:300px[;}]/);
  assert.match(shell, /\.app\.side-narrow\{--side-w:3\.5rem\}/);
  // The collapsed rail and its toggle are declared.
  assert.match(shell, /\.side\.is-narrow[^{]*\{/);
  assert.match(shell, /\.side \.side-collapse\{color:var\(--faint\)\}/);
  // Allow main column flex shrinking to prevent horizontal window overflow.
  assert.match(shell, /\.main\{[^}]*min-width:0[^}]*\}/);
  assert.match(
    shell,
    /\.path\{[^}]*padding:8px 20px;[^}]*border-bottom:1px solid var\(--line-soft\)/,
  );
  assert.match(shell, /\.header-action\{[^}]*margin-left:auto;[^}]*flex:none/);
  assert.match(
    shell,
    /\.settings-main>\.settings-inset\+\.band\{margin-top:6px\}/,
  );
  // Title row flex shrinkage preserves action button visibility in headers.
  assert.match(shell, /\.loc\{[^}]*flex:1 1 auto/);
  assert.match(shell, /\.loc-copy\{[^}]*white-space:nowrap/);
  assert.match(shell, /\.header-action \.btn\.cap\{height:28px;font-size:13px/);
  assert.match(shell, /\.loc h1\{display:inline;/);
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
  assert.match(shell, /\.tile \.qa\{[^}]*right:6px;bottom:6px;/);
  assert.match(
    shell,
    /\.folder-split\{[^}]*grid-template-columns:236px minmax\(0,1fr\)/,
  );
  assert.match(
    shell,
    /\.tpane\{[^}]*background:var\(--main-surface\)[^}]*padding:4px 6px 8px/,
  );
  assert.match(shell, /\.tpane \.fn\{[^}]*font-weight:500/);
  assert.doesNotMatch(shell, /\.tpane \.fn\.on\{[^}]*font-weight/);
  assert.match(shell, /\.twist\{[^}]*left:calc\(6px \+ var\(--d,0\) \* 18px\)/);
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

test('desktop titlebar applies left padding to clear macOS window controls', async () => {
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
