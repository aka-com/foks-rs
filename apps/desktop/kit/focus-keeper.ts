/**
 * Keeps the keyboard focus off `<body>`.
 *
 * A control that unmounts or becomes disabled while it has the focus (a row
 * that was deleted, a Show button that turns into Hide, a sidebar toggle
 * that moves to the other bar, a popup closed by a click on blank space,
 * a list replaced by navigation) leaves the focus on `<body>`, where the
 * next Tab starts again from the top of the window and VoiceOver loses its
 * place. The keeper notices that and moves the focus, in this order, to:
 *
 * 1. a target a caller asked for with `restoreFocusIfLost(...)`;
 * 2. when the loss follows a navigation, the control that had the focus the
 *    last time this place was shown (Back returns to the row the user left
 *    from);
 * 3. the element now standing where the lost one stood: the same
 *    `data-focus-key`, `id`, `data-row-key` (rows and their hover actions)
 *    or `data-place` (sidebar rows);
 * 4. the place's landing target: the selected list option, the first list
 *    row, the page heading, then `.main` itself.
 *
 * It never takes the focus from an element that still holds it, and leaves a
 * click on blank space alone while the control it left is still usable.
 * While a modal dialog is open, only targets inside that dialog qualify.
 *
 * Usage for screens: give a control that is swapped for another (Show and
 * Hide, Edit and Done) the same `data-focus-key`, or call
 * `restoreFocusIfLost(next)` after a mutation with the element (or selector)
 * that should get the focus if the current one goes away.
 */

type Candidate =
  | HTMLElement
  | string
  | null
  | undefined
  | (() => HTMLElement | null | undefined);

/** Where the focus lands in a place when nothing more specific applies. */
const LANDING = [
  '.main [role="listbox"] [role="option"][aria-selected="true"]',
  '.main .row.sel[data-row-key]',
  '.main [role="listbox"] [role="option"][tabindex="0"]',
  '.main [role="tablist"] [role="tab"][aria-selected="true"]',
  '.main .rows [data-row-key]',
  '.main [role="option"]',
  '.main h1[tabindex="-1"]',
  '.main .list-head h2[tabindex="-1"]',
  '.main',
] as const;

interface Lost {
  /** Selectors that find the element now standing where the lost one stood. */
  identity: string[];
  element: HTMLElement;
  /**
   * The element was disabled, hidden or removed while it held the focus (a
   * button that disables itself while its action runs), as opposed to the
   * user clicking blank space. It may be usable again by the time the keeper
   * looks, and the focus still has to come back.
   */
  taken?: boolean;
  /** The user moved the focus off it with a pointer press on blank space. */
  released?: boolean;
}

let last: Lost | null = null;
/**
 * Targets callers asked for. A request outlives checks that find the focus
 * where it should be (a dialog returning it to its opener first, a refresh
 * that removes the row later) and lapses after `REQUEST_MS`.
 */
let requested: { candidates: Candidate[]; until: number }[] = [];
const REQUEST_MS = 4000;
let timer: ReturnType<typeof setTimeout> | null = null;
let context = '';
let contextChanged = false;
/**
 * Whether the user has pressed a key or the pointer, or the focus has been
 * placed, since the keeper started. Until then a change of place is the
 * window settling on its first location (a location the shell completes once
 * the snapshot arrives), not a navigation, and the focus stays where the
 * platform put it.
 */
let engaged = false;
const remembered = new Map<string, string[]>();

function quoted(value: string): string {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

/** Selectors that find `element`, or its replacement, after a re-render. */
export function focusIdentity(element: Element): string[] {
  const identity: string[] = [];
  const key = element.getAttribute('data-focus-key');
  if (key) identity.push(`[data-focus-key=${quoted(key)}]`);
  if (element.id) identity.push(`[id=${quoted(element.id)}]`);
  const row = element.closest('[data-row-key]');
  if (row)
    identity.push(
      `[data-row-key=${quoted(row.getAttribute('data-row-key') ?? '')}]`,
    );
  const place = element.closest('[data-place]');
  if (place)
    identity.push(
      `[data-place=${quoted(place.getAttribute('data-place') ?? '')}]`,
    );
  return identity;
}

/** Whether the focus has nowhere useful to be: `<body>`, or a dead element. */
export function focusLost(): boolean {
  if (typeof document === 'undefined') return false;
  const active = document.activeElement;
  return (
    !active ||
    active === document.body ||
    active === document.documentElement ||
    !active.isConnected ||
    (active instanceof HTMLElement && !usable(active))
  );
}

function topDialog(): HTMLElement | null {
  const dialogs = Array.from(
    document.querySelectorAll<HTMLElement>(
      '[role="dialog"][aria-modal="true"], [role="alertdialog"][aria-modal="true"]',
    ),
  ).filter((dialog) => !dialog.inert && !dialog.closest('[inert]'));
  return dialogs.at(-1) ?? null;
}

/** Connected, enabled, not inert or hidden, and able to take the focus. */
function usable(element: HTMLElement): boolean {
  if (!element.isConnected) return false;
  if ((element as HTMLButtonElement).disabled) return false;
  if (element.closest('[inert], [aria-hidden="true"], [hidden]')) return false;
  return (
    element.tabIndex >= 0 ||
    element.hasAttribute('tabindex') ||
    /^(A|BUTTON|INPUT|SELECT|TEXTAREA|SUMMARY)$/.test(element.tagName)
  );
}

/** Usable, and not behind the open modal dialog (popups portal outside it). */
function focusable(element: HTMLElement): boolean {
  if (!usable(element)) return false;
  const dialog = topDialog();
  return (
    !dialog ||
    dialog.contains(element) ||
    element.closest('[data-popup]') !== null
  );
}

function resolve(candidate: Candidate): HTMLElement | null {
  const value = typeof candidate === 'function' ? candidate() : candidate;
  if (!value) return null;
  if (typeof value !== 'string') return focusable(value) ? value : null;
  for (const element of document.querySelectorAll<HTMLElement>(value))
    if (focusable(element)) return element;
  return null;
}

/**
 * Where the focus lands in a modal dialog whose focused control went away
 * (a Start button replaced by the phrase it fetched, a submit that failed):
 * the field in error, the dialog's own initial control, the first control in
 * its body, its default button, any other footer button (Cancel, when the
 * failure left nothing else to do), then the dialog itself.
 */
const DIALOG_LANDING = [
  '[aria-invalid="true"]',
  '[data-dialog-autofocus="true"], [data-sheet-autofocus="true"]',
  '.sb input, .sb select, .sb textarea, .sb button, .sb [tabindex="0"]',
  '.ft .btn.primary',
  '.ft .btn',
] as const;

function landing(): HTMLElement | null {
  const dialog = topDialog();
  if (dialog) {
    for (const selector of DIALOG_LANDING)
      for (const element of dialog.querySelectorAll<HTMLElement>(selector))
        if (focusable(element)) return element;
    return dialog;
  }
  for (const selector of LANDING) {
    const target = resolve(selector);
    if (target) return target;
  }
  return null;
}

function check(): void {
  timer = null;
  const now = Date.now();
  requested = requested.filter((request) => request.until > now);
  const asked = requested.flatMap((request) => request.candidates);
  const navigated = contextChanged;
  contextChanged = false;
  if (!focusLost()) return;
  requested = [];
  // A click on blank space leaves a usable control behind; that is the
  // user's choice. Nothing focused yet (the window just opened) is the
  // platform's. Only a control that went away, a navigation or a caller's
  // request moves the focus.
  if (
    !asked.length &&
    !navigated &&
    (!last ||
      (!last.taken && last.element.isConnected && focusable(last.element)))
  )
    return;
  const candidates: Candidate[] = [
    ...asked,
    ...(navigated ? (remembered.get(context) ?? []) : []),
    ...(last?.identity ?? []),
  ];
  for (const candidate of candidates) {
    const target = resolve(candidate);
    if (target) {
      target.focus();
      return;
    }
  }
  landing()?.focus({ preventScroll: true });
}

function schedule(): void {
  if (timer !== null || typeof window === 'undefined') return;
  // After React's commit and its effects, which may place the focus
  // themselves (a list focusing its selected row, a dialog restoring its
  // opener). The keeper only fills in when none of them did.
  timer = setTimeout(check, 0);
}

/**
 * Asks for the focus to move to the first usable candidate if, once the
 * current update settles, it has been lost. Callers use it after a mutation
 * (the next row after a delete), a light-dismissed popup (its trigger) or an
 * Escape that takes the focused control away.
 */
export function restoreFocusIfLost(...candidates: Candidate[]): void {
  requested.push({ candidates, until: Date.now() + REQUEST_MS });
  schedule();
}

/**
 * Names the place on screen, such as an encoded location. When it changes and
 * the focus was left behind in the old place, the keeper returns to the
 * control last focused in the new one, or lands there.
 */
export function setFocusContext(key: string): void {
  if (key === context) return;
  context = key;
  if (!engaged) return;
  contextChanged = true;
  schedule();
}

/** Starts the keeper for this document; returns the stop function. */
export function mountFocusKeeper(
  root: HTMLElement = document.body,
): () => void {
  // The window opening is not a navigation: the focus starts where the
  // platform puts it.
  contextChanged = false;
  engaged = false;
  const onFocusIn = (event: FocusEvent): void => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || target === document.body) return;
    engaged = true;
    // A pointer press is the user choosing where the focus goes; earlier
    // requests no longer apply.
    if (pressing) requested = [];
    const identity = focusIdentity(target);
    last = { element: target, identity };
    // Remember controls in the page's own columns; the sidebar keeps its
    // focus across navigation, and dialogs and popups do not survive it.
    if (target.closest('.main, .details')) remembered.set(context, identity);
  };
  let pressing = false;
  const onKeyDown = (): void => {
    engaged = true;
  };
  const onPointerDown = (): void => {
    engaged = true;
    pressing = true;
    setTimeout(() => (pressing = false), 0);
  };
  const onFocusOut = (event: FocusEvent): void => {
    if (event.relatedTarget !== null) return;
    if (pressing && last?.element === event.target) last.released = true;
    schedule();
  };
  const observer =
    typeof MutationObserver === 'undefined'
      ? null
      : new MutationObserver(() => {
          if (!last) return;
          if (
            !last.released &&
            (!last.element.isConnected || !usable(last.element))
          )
            last.taken = true;
          if (!last.element.isConnected || focusLost()) schedule();
        });
  observer?.observe(root, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['disabled', 'inert', 'hidden'],
  });
  document.addEventListener('focusin', onFocusIn);
  document.addEventListener('focusout', onFocusOut);
  document.addEventListener('pointerdown', onPointerDown, true);
  document.addEventListener('keydown', onKeyDown, true);
  return () => {
    observer?.disconnect();
    document.removeEventListener('focusin', onFocusIn);
    document.removeEventListener('focusout', onFocusOut);
    document.removeEventListener('pointerdown', onPointerDown, true);
    document.removeEventListener('keydown', onKeyDown, true);
    if (timer !== null) clearTimeout(timer);
    timer = null;
    last = null;
    requested = [];
    remembered.clear();
    context = '';
    contextChanged = false;
    engaged = false;
  };
}

/**
 * Moves the focus to `candidate` once it is on screen, whatever has the focus
 * then: for a command whose whole point is to land somewhere, such as opening
 * an item from the search palette. Gives up after about half a second.
 */
export function focusWhenRendered(candidate: Candidate, tries = 30): void {
  if (typeof window === 'undefined') return;
  setTimeout(() => {
    const target = resolve(candidate);
    if (target) target.focus();
    else if (tries > 0) focusWhenRendered(candidate, tries - 1);
  }, 16);
}

/** The list row for `key` (`store|path`) in the page, if it is drawn. */
export function rowByKey(key: string): () => HTMLElement | undefined {
  return () =>
    Array.from(
      document.querySelectorAll<HTMLElement>('.main [data-row-key]'),
    ).find((row) => row.dataset.rowKey === key);
}
