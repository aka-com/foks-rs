/**
 * The add and rename server sheets rendered from their own modules under
 * features/settings/servers: what each accepts, and what it asks the bridge
 * for.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactNode } from 'react';

import type { Bridge, CheckedProfileResponse } from '../src/bridge';
import type { Server } from '../src/model';
import { installDom } from './lib/dom-harness';

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
  const { AddServerSheet } =
    await import('../src/features/settings/servers/add-server-sheet');
  const { RenameServerSheet } =
    await import('../src/features/settings/servers/rename-server-sheet');
  const { FIXTURE } = await import('../src/mock/fixture');
  const { mockBridge } = await import('../src/mock/mock-bridge');
  const { OverlayProvider } = await import('../kit/overlay-primitives');
  const portalRoot = document.getElementById('overlays');
  assert.ok(portalRoot);
  const personal = FIXTURE.servers.find(
    (server): server is Server => server.profileName === 'personal',
  );
  assert.ok(personal, 'the fixture holds the personal server');
  return {
    AddServerSheet,
    RenameServerSheet,
    personal,
    bridge: (overrides: Partial<Bridge>): Bridge => ({
      ...mockBridge(FIXTURE),
      ...overrides,
    }),
    render: (node: ReactNode) =>
      ui.render(
        createElement(OverlayProvider, {
          backgroundRef: { current: null },
          portalRoot,
          children: node,
        }),
      ),
  };
}

test('Add server waits for a valid name and an address, and adds what was typed', async () => {
  const h = await harness();
  const checked: [string, string][] = [];
  const added: string[] = [];
  const rendered = h.render(
    createElement(h.AddServerSheet, {
      bridge: h.bridge({
        checkAndAddProfile: async (profile, probe) => {
          checked.push([profile, probe]);
          return { profile } as CheckedProfileResponse;
        },
      }),
      onClose: () => {},
      onAdded: async (profile) => {
        added.push(profile);
      },
      onError: (error) => assert.fail(String(error)),
    }),
  );
  const add = rendered.getByRole('button', {
    name: 'Add server',
  }) as HTMLButtonElement;
  assert.equal(add.disabled, true);
  ui.fireEvent.change(rendered.getByLabelText('Server name'), {
    target: { value: 'new foks' },
  });
  ui.fireEvent.change(rendered.getByLabelText('Address'), {
    target: { value: ' foks.example.com ' },
  });
  // A server name is a profile name: no spaces.
  assert.equal(add.disabled, true);
  ui.fireEvent.change(rendered.getByLabelText('Server name'), {
    target: { value: 'new-foks' },
  });
  assert.equal(add.disabled, false);
  await ui.act(async () => {
    ui.fireEvent.click(add);
  });
  assert.deepEqual(checked, [['new-foks', 'foks.example.com']]);
  assert.deepEqual(added, ['new-foks']);
});

test('Rename server clears the label for an empty name and reports a mismatched answer', async () => {
  const h = await harness();
  const labels: [string, string | null][] = [];
  const errors: string[] = [];
  let renamed = 0;
  const rendered = h.render(
    createElement(h.RenameServerSheet, {
      server: h.personal,
      bridge: h.bridge({
        setServerLabel: async (profile, label) => {
          labels.push([profile, label]);
          return {
            profile,
            label: labels.length > 1 ? 'other' : label,
            changed: true,
          };
        },
      }),
      onClose: () => {},
      onRenamed: async () => {
        renamed += 1;
      },
      onError: (error) => errors.push((error as Error).message),
    }),
  );
  const name = rendered.getByLabelText('Display name') as HTMLInputElement;
  assert.equal(name.value, 'Personal server');
  ui.fireEvent.change(name, { target: { value: '  ' } });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Save' }));
  });
  ui.fireEvent.change(name, { target: { value: 'Home' } });
  await ui.act(async () => {
    ui.fireEvent.click(rendered.getByRole('button', { name: 'Save' }));
  });
  assert.deepEqual(labels, [
    ['personal', null],
    ['personal', 'Home'],
  ]);
  assert.equal(renamed, 1);
  assert.deepEqual(errors, [
    'set_server_label returned a different server label.',
  ]);
});
