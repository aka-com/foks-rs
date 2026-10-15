/**
 * Loads every screen module the shell defers to a dynamic import, so a render
 * test that opens one finds the screen rather than its loading pane. Without
 * it, the first visit to each screen in a test file waits on tsx compiling
 * the screen's modules, which can outlast a `waitFor` deadline.
 *
 * The modules load on call, so a render test can import this file before
 * `installDom`.
 */
export async function loadDeferredScreens(): Promise<void> {
  const { preloadFirstRun, preloadVaultScreens } =
    await import('../../src/app/deferred-screens');
  await Promise.all([preloadFirstRun(), preloadVaultScreens()]);
}
