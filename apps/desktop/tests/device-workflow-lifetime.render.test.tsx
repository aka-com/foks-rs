import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, StrictMode, useEffect } from 'react';
import { createServer } from 'vite';
import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/', timers: true, act: true });

test('Strict Mode cannot revive device workflow completions from its first setup', async () => {
  const ui = await import('@testing-library/react');
  const vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  try {
    const { useDeviceOperation } = (await vite.ssrLoadModule(
      '/src/screens/devices/use-device-operation.ts',
    )) as typeof import('../src/screens/devices/use-device-operation');
    const { ToastProvider, ToastController } = (await vite.ssrLoadModule(
      '/kit/toasts.tsx',
    )) as typeof import('../kit/toasts');
    const captures: (() => boolean)[] = [];
    function Probe() {
      const operation = useDeviceOperation('profile/account');
      useEffect(() => {
        captures.push(operation.capture());
      }, [operation]);
      return null;
    }
    const view = ui.render(
      createElement(
        StrictMode,
        null,
        createElement(ToastProvider, {
          controller: new ToastController(),
          children: createElement(Probe),
        }),
      ),
    );
    assert.equal(captures.length, 2);
    assert.equal(captures[0](), false);
    assert.equal(captures[1](), true);
    view.unmount();
    assert.equal(captures[1](), false);
  } finally {
    ui.cleanup();
    await vite.close();
  }
});
