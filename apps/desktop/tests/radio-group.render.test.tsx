import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, useState } from 'react';
import type { ReactNode } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom, settle } from './lib/dom-harness';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let components: typeof import('../src/components');
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  components = (await vite.ssrLoadModule(
    '/src/components/index.ts',
  )) as typeof components;
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

/** A sheet with a name field, three radios (the third `off`) and a default. */
function Harness({ onCreate }: { onCreate: (choice: string) => void }) {
  const { Sheet, RadioGroup, RadioCard, Button, Inset, Field } = components;
  const [choice, setChoice] = useState('one');
  const [name, setName] = useState('');
  const card = (id: string, off = false): ReactNode =>
    createElement(RadioCard, {
      key: id,
      title: id,
      detail: off ? 'Not available here' : undefined,
      off,
      selected: choice === id,
      onSelect: () => setChoice(id),
    });
  return createElement(Sheet, {
    title: 'Create',
    footer: createElement(
      Button,
      { variant: 'primary', onClick: () => onCreate(choice) },
      'Create',
    ),
    children: [
      createElement(Inset, {
        key: 'fields',
        children: createElement(Field, {
          label: 'Name',
          value: name,
          onChange: setName,
        }),
      }),
      createElement(RadioGroup, {
        key: 'radios',
        label: 'Kind',
        children: [card('one'), card('two'), card('three', true)],
      }),
    ],
  });
}

function radios(): HTMLElement[] {
  return ui.screen.getAllByRole('radio');
}

test('a radio group is one Tab stop: the checked radio', async () => {
  ui.render(createElement(Harness, { onCreate: () => {} }));
  assert.deepEqual(
    radios().map((radio) => radio.tabIndex),
    [0, -1, -1],
  );
  ui.act(() => radios()[1].click());
  // The group re-picks its stop when a radio's `aria-checked` changes.
  await settle();
  assert.deepEqual(
    radios().map((radio) => radio.tabIndex),
    [-1, 0, -1],
  );
});

test('arrow keys move to the next radio, wrapping, and choose it', () => {
  ui.render(createElement(Harness, { onCreate: () => {} }));
  radios()[0].focus();
  ui.fireEvent.keyDown(radios()[0], { key: 'ArrowDown' });
  assert.equal(document.activeElement, radios()[1]);
  assert.equal(radios()[1].getAttribute('aria-checked'), 'true');
  // An `off` radio takes the focus so its reason is read, but is not chosen.
  ui.fireEvent.keyDown(radios()[1], { key: 'ArrowRight' });
  assert.equal(document.activeElement, radios()[2]);
  assert.equal(radios()[2].getAttribute('aria-checked'), 'false');
  assert.equal(radios()[1].getAttribute('aria-checked'), 'true');
  ui.fireEvent.keyDown(radios()[2], { key: 'ArrowDown' });
  assert.equal(document.activeElement, radios()[0]);
  assert.equal(radios()[0].getAttribute('aria-checked'), 'true');
  ui.fireEvent.keyDown(radios()[0], { key: 'ArrowUp' });
  assert.equal(document.activeElement, radios()[2]);
});

test('Return on a radio presses the sheet’s default button', () => {
  const created: string[] = [];
  ui.render(
    createElement(Harness, { onCreate: (choice) => created.push(choice) }),
  );
  ui.act(() => radios()[1].click());
  radios()[1].focus();
  ui.fireEvent.keyDown(radios()[1], { key: 'Enter' });
  assert.deepEqual(created, ['two']);
});

test('a field with an error is invalid and described by the message', () => {
  const { Field, Inset } = components;
  ui.render(
    createElement(Inset, {
      children: createElement(Field, {
        label: 'Path',
        value: '/logins/github.com',
        onChange: () => {},
        error: '/logins/github.com already exists.',
      }),
    }),
  );
  const input = ui.screen.getByLabelText('Path');
  assert.equal(input.getAttribute('aria-invalid'), 'true');
  const message = document.getElementById(
    input.getAttribute('aria-describedby') ?? '',
  );
  assert.equal(message?.textContent, '/logins/github.com already exists.');
  assert.equal(message?.getAttribute('role'), 'alert');
});
