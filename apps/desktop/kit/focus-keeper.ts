/**
 * Prevents keyboard focus from falling back to `<body>`.
 *
 * When a focused control unmounts or becomes disabled (for example, when a row
 * is deleted, a Show button switches to Hide, a sidebar toggle moves to
 * another bar, a popup closes on an outside click, or navigation replaces a
 * list), focus falls back to `<body>`. From there, the next Tab restarts from
 * the top of the window and VoiceOver loses its position. The focus keeper
 * detects this and moves focus to the first available target in this order:
 *
 * 1. A target requested via `restoreFocusIfLost(...)`.
 * 2. After navigation, the control last focused in the destination view (so
 *    Back returns to the previously focused row).
 * 3. The replacement element with the same `data-focus-key`, `id`,
 *    `data-row-key` (rows and their hover actions), or `data-place` (sidebar
 *    rows).
 * 4. The view's default landing target: the selected list option, the first
 *    list row, the page heading, or `.main`.
 *
 * The focus keeper leaves focus unchanged when the active element remains
 * focusable, and does not move focus after a click on blank space while the
 * previously focused control remains usable. While a modal dialog is open,
 * only targets inside that dialog are eligible.
 *
 * Usage: assign the same `data-focus-key` to controls that swap in place
 * (such as Show/Hide or Edit/Done), or call `restoreFocusIfLost(next)` after a
 * mutation with the element or selector to focus if the current control
 * unmounts.
 */

type Candidate =
  | HTMLElement
  | string
  | null
  | undefined
  | (() => HTMLElement | null | undefined);

/** Default focus selectors in priority order when no more specific target applies. */
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
}

let last: Lost | null = null;
/**
 * Focus targets requested by callers. Requests persist across checks where
 * focus is already valid (for example, when a dialog restores focus to its
 * opener before a refresh removes the row) and expire after `REQUEST_MS`.
 */
let requested: { candidates: Candidate[]; until: number }[] = [];
const REQUEST_MS = 4000;
let timer: ReturnType<typeof setTimeout> | null = null;
let context = '';
let contextChanged = false;
const remembered = new Map<string, string[]>();

function quoted(value: string): string {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

/** Returns CSS selectors that match `element` or its replacement after a re-render. */
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

/** Returns whether `element` is connected, enabled, not inert or hidden, and focusable. */
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

/** Returns whether `element` is usable and not blocked by an open modal dialog (portaled popups remain eligible). */
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

function landing(): HTMLElement | null {
  const dialog = topDialog();
  if (dialog) return dialog;
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
    (!last || (last.element.isConnected && focusable(last.element)))
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
  // Defer until after React commits and runs effects that manage focus
  // directly (such as a list focusing its selected row or a dialog restoring
  // its opener). The keeper intervenes only if focus is still lost.
  timer = setTimeout(check, 0);
}

/**
 * Moves focus to the first focusable candidate if focus is lost after the
 * current update settles. Callers invoke this after mutations (such as
 * focusing the neighbouring row after a delete), light-dismissed popups
 * (returning to the trigger), or Escape handlers that unmount the focused
 * control.
 */
export function restoreFocusIfLost(...candidates: Candidate[]): void {
  requested.push({ candidates, until: Date.now() + REQUEST_MS });
  schedule();
}

/**
 * Sets the active focus context key (such as an encoded location). When the
 * context changes and focus was in the previous view, the keeper restores the
 * control last focused in the new context or falls back to its landing target.
 */
export function setFocusContext(key: string): void {
  if (key === context) return;
  context = key;
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
  const onFocusIn = (event: FocusEvent): void => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || target === document.body) return;
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
  const onPointerDown = (): void => {
    pressing = true;
    setTimeout(() => (pressing = false), 0);
  };
  const onFocusOut = (event: FocusEvent): void => {
    if (event.relatedTarget === null) schedule();
  };
  const observer =
    typeof MutationObserver === 'undefined'
      ? null
      : new MutationObserver(() => {
          if (last && (!last.element.isConnected || focusLost())) schedule();
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
  return () => {
    observer?.disconnect();
    document.removeEventListener('focusin', onFocusIn);
    document.removeEventListener('focusout', onFocusOut);
    document.removeEventListener('pointerdown', onPointerDown, true);
    if (timer !== null) clearTimeout(timer);
    timer = null;
    last = null;
    requested = [];
    remembered.clear();
    context = '';
    contextChanged = false;
  };
}

/**
 * Focuses `candidate` once it renders, regardless of current focus. Callers
 * use this for commands that explicitly move focus to a destination (such as
 * opening an item from the search palette). Stops polling after `tries`
 * attempts (~500ms).
 */
export function focusWhenRendered(candidate: Candidate, tries = 30): void {
  if (typeof window === 'undefined') return;
  setTimeout(() => {
    const target = resolve(candidate);
    if (target) target.focus();
    else if (tries > 0) focusWhenRendered(candidate, tries - 1);
  }, 16);
}

/** Returns a resolver for the list row matching `key` (`store|path`) in `.main`. */
export function rowByKey(key: string): () => HTMLElement | undefined {
  return () =>
    Array.from(
      document.querySelectorAll<HTMLElement>('.main [data-row-key]'),
    ).find((row) => row.dataset.rowKey === key);
}
