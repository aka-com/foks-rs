/**
 * The header mark of `SheetDialog`: the `icon` shorthand draws a server mark,
 * tinted for a danger sheet, and an explicit `glyph` takes precedence.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import type { ReactNode } from 'react';

import { installDom } from './lib/dom-harness';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
});

let ui: typeof import('@testing-library/react');
let components: typeof import('../src/components');
let overlays: typeof import('../kit/overlay-primitives');
test.before(async () => {
  ui = await import('@testing-library/react');
  components = await import('../src/components');
  overlays = await import('../kit/overlay-primitives');
});
test.afterEach(() => {
  ui.cleanup();
});

/** The sheet portals into the overlay root, so it needs a provider. */
function render(sheet: ReactNode): void {
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  ui.render(
    <overlays.OverlayProvider
      backgroundRef={{ current: null }}
      portalRoot={portalRoot}
    >
      {sheet}
    </overlays.OverlayProvider>,
  );
}

function header(name: string): Element {
  const dialog = ui.screen.getByRole('dialog', { name });
  const hd = dialog.querySelector('.sheet > .hd');
  assert.ok(hd, 'the sheet has a header');
  return hd;
}

test('icon draws a server mark holding that icon beside the title', () => {
  render(
    <components.SheetDialog
      title="Add a server"
      icon="server"
      onClose={() => {}}
    >
      Body
    </components.SheetDialog>,
  );
  const mark = header('Add a server').querySelector(':scope > .server-mark');
  assert.ok(mark, 'the header starts with a server mark');
  assert.equal(mark.className, 'server-mark');
  assert.ok(mark.querySelector('svg.ic'), 'the mark holds an icon');
});

test('a danger sheet tints the server mark', () => {
  render(
    <components.SheetDialog
      title="Remove it?"
      icon="trash"
      danger
      onClose={() => {}}
    >
      Body
    </components.SheetDialog>,
  );
  const dialog = ui.screen.getByRole('alertdialog', { name: 'Remove it?' });
  const mark = dialog.querySelector('.sheet > .hd > .server-mark');
  assert.ok(mark);
  assert.equal(mark.className, 'server-mark danger');
});

test('an explicit glyph replaces the icon mark', () => {
  render(
    <components.SheetDialog
      title="Custom"
      icon="settings"
      glyph={<span className="custom-mark" />}
      onClose={() => {}}
    >
      Body
    </components.SheetDialog>,
  );
  const hd = header('Custom');
  assert.ok(hd.querySelector(':scope > .custom-mark'));
  assert.equal(hd.querySelector('.server-mark'), null);
});

test('a sheet with neither icon nor glyph draws no mark', () => {
  render(
    <components.SheetDialog title="Plain" onClose={() => {}}>
      Body
    </components.SheetDialog>,
  );
  const hd = header('Plain');
  assert.equal(hd.firstElementChild?.className, 't');
});
