import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';
import { decodeCommandAck } from '../src/bridge';
installDom({
  url: 'http://localhost/',
  body: '<div id="root"></div>',
  timers: true,
});
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

test('admin acknowledgements cannot return bearer URLs to the main webview', () => {
  assert.deepEqual(decodeCommandAck({ ok: true }), { ok: true });
  assert.throws(() => decodeCommandAck({ ok: true, url: 'secret' }));
});
test('admin configuration and opening are distinct explicit actions and PIN is cleared', async () => {
  const { AdminPanel } = (await vite.ssrLoadModule(
    '/src/components/admin-panel.tsx',
  )) as typeof import('../src/components/admin-panel');
  const { mockBridge } = (await vite.ssrLoadModule(
    '/src/mock-bridge.ts',
  )) as typeof import('../src/mock-bridge');
  const calls: unknown[][] = [];
  const bridge = {
    ...mockBridge(),
    configureWebAdmin: async (...args: [string, string, string]) => {
      calls.push(args);
      return { ok: true as const };
    },
    openWebAdmin: async (...args: [string, string, string | null]) => {
      calls.push(args);
      throw new Error('Host administration unsupported');
    },
  };
  const r = ui.render(
    createElement(AdminPanel, { bridge, profile: 'local', account: 'work' }),
  );
  ui.fireEvent.change(r.getByLabelText('Admin HTTPS address'), {
    target: { value: 'https://admin.example/' },
  });
  ui.fireEvent.click(r.getByText('Save admin destination'));
  await ui.waitFor(() => assert.ok(r.queryByText('Admin destination saved.')));
  assert.deepEqual(calls, [['local', 'work', 'https://admin.example/']]);
  ui.fireEvent.change(r.getByLabelText('Admin security key PIN, if needed'), {
    target: { value: '654321' },
  });
  ui.fireEvent.click(r.getByText('Open host administration'));
  await ui.waitFor(() => assert.ok(r.queryByRole('alert')));
  assert.deepEqual(calls[1], ['local', 'work', '654321']);
  assert.equal(
    (r.getByLabelText('Admin security key PIN, if needed') as HTMLInputElement)
      .value,
    '',
  );
  r.rerender(
    createElement(AdminPanel, { bridge, profile: 'local', account: 'other' }),
  );
  await ui.waitFor(() => assert.ok(r.queryByRole('alert') === null));
});
