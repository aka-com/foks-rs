/**
 * The screens the shell loads on demand, each in its own build chunk.
 *
 * The startup JavaScript holds the shell, the Files screen and the chat
 * providers; first-run setup, the Chat tab, Teams, Devices, Settings and team
 * settings load by dynamic import the first time one renders. While a chunk
 * loads, the nearest `<Suspense>` shows {@link ScreenLoading}. Once a
 * screen's module has loaded, the screen renders synchronously, without
 * suspending again. The build fails when the startup JavaScript exceeds the
 * budget in bundle-budget.ts.
 *
 * {@link usePreloadVaultScreens} loads the vault screens' chunks in idle time
 * after the screen router mounts, so later navigation does not wait on a
 * chunk. First-run setup is loaded only when it is shown.
 */

import { createElement, use, useEffect } from 'react';
import type { ComponentType, ReactNode } from 'react';

export interface DeferredScreen<P extends object> {
  Component: ComponentType<P>;
  /** Starts loading the screen's chunk; later calls share the first load. */
  preload: () => Promise<unknown>;
}

/**
 * A component that renders the one `load` resolves to, suspending until the
 * first load completes. `name` is its display name in React's tools.
 */
export function deferredScreen<P extends object>(
  name: string,
  load: () => Promise<ComponentType<P>>,
): DeferredScreen<P> {
  let loaded: ComponentType<P> | undefined;
  let pending: Promise<ComponentType<P>> | undefined;
  // A failed load stays failed, as with React.lazy: the render that reads it
  // throws to the screen's error boundary instead of retrying in a loop.
  const preload = () =>
    (pending ??= load().then((screen) => (loaded = screen)));
  function Deferred(props: P): ReactNode {
    return createElement(loaded ?? use(preload()), props);
  }
  Deferred.displayName = name;
  return { Component: Deferred, preload };
}

const firstRun = deferredScreen('FirstRunExperience', () =>
  import('../features/first-run/first-run-screen').then(
    (m) => m.FirstRunExperience,
  ),
);
const chatTab = deferredScreen('ChatTab', () =>
  import('../features/chat/chat-tab').then((m) => m.ChatTab),
);
const teams = deferredScreen('TeamsScreen', () =>
  import('../features/teams/teams-screen').then((m) => m.TeamsScreen),
);
const devices = deferredScreen('DevicesScreen', () =>
  import('../features/devices/devices-screen').then((m) => m.DevicesScreen),
);
const settings = deferredScreen('SettingsScreen', () =>
  import('../features/settings/settings-screen').then((m) => m.SettingsScreen),
);
const groupSettings = deferredScreen('GroupSettingsScreen', () =>
  import('../features/teams/groups-screen').then((m) => m.GroupSettingsScreen),
);

export const FirstRunExperience = firstRun.Component;
export const ChatTab = chatTab.Component;
export const TeamsScreen = teams.Component;
export const DevicesScreen = devices.Component;
export const SettingsScreen = settings.Component;
export const GroupSettingsScreen = groupSettings.Component;

const VAULT_SCREENS = [chatTab, teams, devices, settings, groupSettings];

/** Loads the chunks of every deferred screen except first-run setup. */
export async function preloadVaultScreens(): Promise<void> {
  await Promise.all(VAULT_SCREENS.map((screen) => screen.preload()));
}

/** Loads the first-run setup chunk. */
export async function preloadFirstRun(): Promise<void> {
  await firstRun.preload();
}

/**
 * How long after the screen router mounts the vault screens start loading
 * when the webview has no `requestIdleCallback`, and the longest an idle
 * callback waits when it has. WebKitGTK, which hosts the app on Linux, does
 * not provide `requestIdleCallback`.
 */
const PRELOAD_DELAY_MS = 1000;

/**
 * Preloads the vault screens once the calling component has mounted, in an
 * idle period where the webview reports one and after a fixed delay where it
 * does not. Preloading is idempotent, so a remount schedules a call that
 * finds the loads already started. A load that fails here is reported by the
 * render that needs it.
 */
export function usePreloadVaultScreens(): void {
  useEffect(() => {
    const preload = () => void preloadVaultScreens().catch(() => undefined);
    if (typeof window.requestIdleCallback === 'function') {
      const handle = window.requestIdleCallback(preload, {
        timeout: PRELOAD_DELAY_MS,
      });
      return () => window.cancelIdleCallback(handle);
    }
    const handle = window.setTimeout(preload, PRELOAD_DELAY_MS);
    return () => window.clearTimeout(handle);
  }, []);
}

/** The pane a deferred screen shows while its chunk loads. */
export function ScreenLoading(): ReactNode {
  return (
    <div className="app-loading" role="status" aria-label="Loading">
      <span className="spin" aria-hidden="true" />
    </div>
  );
}
