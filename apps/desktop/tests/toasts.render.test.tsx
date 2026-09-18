import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
let vite: ViteDevServer;
let ToastController: typeof import('../kit/toasts').ToastController;
let ToastProvider: typeof import('../kit/toasts').ToastProvider;
test.before(async () => {
  ui = await import('@testing-library/react');
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ ToastController, ToastProvider } =
    await vite.ssrLoadModule('/kit/toasts.tsx'));
});
test.afterEach(() => ui.cleanup());
test.after(async () => vite.close());

const liveRegions =
  '[role="status"], [role="alert"], [aria-live="polite"], [aria-live="assertive"]';

test('information and warning toasts own independent live regions in presentation order', () => {
  const controller = new ToastController();
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  ui.act(() => {
    controller.show('Saved');
    controller.show('Refresh failed', { tone: 'warning' });
    controller.show('Copied');
  });
  const host = rendered.container.querySelector('#toasts')!;
  assert.equal(host.getAttribute('role'), null);
  assert.equal(host.getAttribute('aria-live'), null);
  assert.deepEqual(
    [...host.querySelectorAll('.toast-message')].map(
      (node) => node.textContent,
    ),
    ['Saved', 'Refresh failed', 'Copied'],
  );
  assert.equal(rendered.getAllByRole('status').length, 2);
  assert.equal(rendered.getAllByRole('alert').length, 1);
  for (const entry of host.querySelectorAll('.toast')) {
    assert.equal(entry.parentElement?.closest(liveRegions), null);
    assert.equal(entry.getAttribute('aria-atomic'), 'true');
    assert.equal(entry.getAttribute('aria-live'), null);
  }
});

test('deduplicated actionable warnings keep one announcement region and usable controls', () => {
  const controller = new ToastController();
  controller.show('Pending refresh', { dedupeKey: 'refresh' });
  const rendered = ui.render(
    createElement(ToastProvider, { controller, children: null }),
  );
  let actions = 0;
  ui.act(() => {
    controller.show('Refresh failed', {
      dedupeKey: 'refresh',
      tone: 'warning',
      action: {
        label: 'Retry',
        onAction: () => {
          actions++;
        },
      },
    });
  });
  assert.equal(rendered.container.querySelectorAll('.toast').length, 1);
  assert.equal(rendered.queryAllByRole('status').length, 0);
  assert.ok(
    rendered.getByRole('alert').textContent?.includes('Refresh failed'),
  );
  ui.fireEvent.click(rendered.getByRole('button', { name: 'Retry' }));
  assert.equal(actions, 1);
  assert.equal(rendered.queryAllByRole('alert').length, 0);
  ui.act(() => {
    controller.show('Another warning', { tone: 'warning' });
  });
  ui.fireEvent.click(
    rendered.getByRole('button', { name: 'Dismiss notification' }),
  );
  assert.equal(rendered.container.querySelectorAll('.toast').length, 0);
});
