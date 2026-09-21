// Apply the saved appearance before the first paint, including the loading screen.
(() => {
  let theme = 'light';
  try {
    const saved = globalThis.localStorage.getItem('appearance');
    if (['light', 'dark', 'system'].includes(saved)) theme = saved;
  } catch {
    /* Storage may be unavailable. */
  }
  globalThis.document.documentElement.dataset.theme =
    theme === 'system'
      ? globalThis.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      : theme;
})();
