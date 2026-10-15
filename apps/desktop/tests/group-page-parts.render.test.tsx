/**
 * The group page's tabs and band rendered from their own modules under
 * screens/groups, without the page around them: which rows and menus each
 * draws, and which sheet a menu choice asks the page to open.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';

import type { AgentSnapshot, Party, Store, TeamStore } from '../src/model';
import type { GroupSheetKind } from '../src/screens/groups/group-sheet';
import { installDom } from './lib/dom-harness';
import { loadTestShell } from './lib/test-shell';

installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div><div id="overlays"></div>',
  timers: true,
  act: true,
});

let ui: typeof import('@testing-library/react');

test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

async function harness() {
  const { FIXTURE } = await import('../src/mock/fixture');
  const { SituationBand } =
    await import('../src/screens/groups/situation-band');
  const { MembersTab } = await import('../src/screens/groups/members-tab');
  const { SettingsTab } = await import('../src/screens/groups/settings-tab');
  const { ToastController, ToastProvider } = await import('../kit/toasts');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const TestShell = await loadTestShell();
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const team = (id: string): TeamStore => {
    const store = FIXTURE.stores.find(
      (candidate): candidate is TeamStore =>
        candidate.kind === 'team' && candidate.id === id,
    );
    assert.ok(store, `the fixture holds ${id}`);
    return store;
  };
  const party = (store: Store, username: string): Party => {
    const found = FIXTURE.parties.find(
      (candidate) =>
        candidate.store === store.id && candidate.username === username,
    );
    assert.ok(found, `the fixture lists ${username} in ${store.id}`);
    return found;
  };
  const render = (node: ReactNode, snapshot: AgentSnapshot = FIXTURE) =>
    ui.render(
      createElement(OverlayProvider, {
        backgroundRef: { current: null },
        portalRoot,
        children: createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(TestShell, { snapshot, children: node }),
        }),
      }),
    );
  return {
    snapshot: FIXTURE,
    SituationBand,
    MembersTab,
    SettingsTab,
    team,
    party,
    render,
  };
}

/** The sheets a tab asked the page to open, with the party each names. */
function sheetRecorder(): {
  onSheet: (sheet: GroupSheetKind | null, party?: Party) => void;
  opened: [GroupSheetKind | null, string | null | undefined][];
} {
  const opened: [GroupSheetKind | null, string | null | undefined][] = [];
  return {
    onSheet: (sheet, party) => opened.push([sheet, party?.username]),
    opened,
  };
}

/**
 * A button of an open menu, by its label: the text a plain item ends with, or
 * the name in bold of an item that also carries a caption.
 */
function menuButton(label: string): HTMLButtonElement {
  const node = [
    ...document.querySelectorAll<HTMLButtonElement>('.menu button'),
  ].find(
    (candidate) =>
      candidate.querySelector('.t b')?.textContent === label ||
      (candidate.textContent ?? '').trim().endsWith(label),
  );
  assert.ok(node, `no menu button labeled ${label}`);
  return node;
}

test('only an ad-hoc team states its fixed membership, and not on Settings', async () => {
  const h = await harness();
  const adhoc = h.team('team:homelab');
  const named = h.team('team:eng');
  const people = h.render(
    createElement(h.SituationBand, { store: adhoc, tab: 'people' }),
  );
  assert.ok(people.getByText('Memberships can’t be changed.'));
  ui.cleanup();
  const settings = h.render(
    createElement(h.SituationBand, { store: adhoc, tab: 'settings' }),
  );
  assert.equal(settings.container.textContent, '');
  ui.cleanup();
  const other = h.render(
    createElement(h.SituationBand, { store: named, tab: 'people' }),
  );
  assert.equal(other.container.textContent, '');
});

test("a member row's Remove asks the page for the remove sheet for that member", async () => {
  const h = await harness();
  const store = h.team('team:eng');
  const dana = h.party(store, 'dana.okafor');
  const sheets = sheetRecorder();
  h.render(
    createElement(h.MembersTab, {
      snapshot: h.snapshot,
      store,
      onSheet: sheets.onSheet,
      menuParty: dana.party_id_hex,
      onRetry: () => {},
      onRetryFederation: () => {},
      onRerun: () => {},
      onRemoveFederatedMembership: () => {},
      onCopy: () => {},
    }),
  );
  ui.fireEvent.click(menuButton('Remove…'));
  assert.deepEqual(sheets.opened, [['remove', 'dana.okafor']]);
});

test('the Settings danger menu lists the members this account can remove, lowest role first', async () => {
  const h = await harness();
  const store = h.team('team:eng');
  const sheets = sheetRecorder();
  h.render(
    createElement(h.SettingsTab, {
      snapshot: h.snapshot,
      store,
      onSheet: sheets.onSheet,
      onNavigate: () => {},
      onCopy: () => {},
      manageable: true,
      rekeyOpen: true,
    }),
  );
  const names = [...document.querySelectorAll('.menu button .t b')].map(
    (node) => node.textContent,
  );
  // The Owner outranks this Admin, and this account does not remove itself.
  assert.deepEqual(names, ['dana.okafor', 'deploy-bot', 'priya.n']);
  ui.fireEvent.click(menuButton('priya.n'));
  assert.deepEqual(sheets.opened, [['remove', 'priya.n']]);
});
