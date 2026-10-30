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
  /** Selectors matching the replacement for the previously focused element. */
  identity: string[];
  element: HTMLElement;
  /**
   * True when the element became disabled, hidden, or unmounted while focused
   * (such as a button disabled while its action runs), rather than blurred by a
   * click on blank space. If the element becomes focusable again before the
   * next check, focus returns to it.
   */
  taken?: boolean;
  /** True when the user moved focus off the element by clicking blank space. */
  released?: boolean;
  /**
   * Up to four ancestor containers of an unmounted control with no direct
   * replacement (such as a row action button removed after completion). Focus
   * moves to the first focusable control in the nearest connected ancestor.
   * Empty for list rows and dialogs, which use dedicated fallback rules.
   */
  around: HTMLElement[];
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
/**
 * True after the first key press, pointer press, or focus event since
 * mounting. Before any user interaction, location changes represent initial
 * route resolution once the snapshot arrives rather than navigation, so
 * initial focus remains unchanged.
 */
let engaged = false;
/** Fallback target focused after the most recent navigation. */
let landed: HTMLElement | null = null;
const remembered = new Map<string, Candidate[]>();

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

function labelOf(element: Element): string {
  return (element.getAttribute('aria-label') ?? element.textContent ?? '')
    .trim()
    .replace(/\s+/g, ' ');
}

/**
 * Matches a control without a key, id, or row attribute by tag name,
 * accessible label, and index within `scopes`.
 */
function sameLabelled(
  element: HTMLElement,
  scopes: readonly string[] = ['.main', '.details'],
): Candidate {
  const label = labelOf(element);
  if (!label) return null;
  const matches = (): HTMLElement[] =>
    Array.from(
      document.querySelectorAll<HTMLElement>(
        scopes.map((scope) => `${scope} ${element.tagName}`).join(', '),
      ),
    ).filter((other) => labelOf(other) === label);
  const index = Math.max(matches().indexOf(element), 0);
  return () => {
    const found = matches();
    return found[index] ?? found[0];
  };
}

/** See `Lost.around`. */
function containersOf(element: HTMLElement): HTMLElement[] {
  if (element.closest('[role="option"], [role="treeitem"], [data-row-key]'))
    return [];
  // Dialogs use `DIALOG_LANDING` instead.
  if (element.closest('[role="dialog"], [role="alertdialog"]')) return [];
  const scope = element.closest<HTMLElement>('.main, .details');
  const around: HTMLElement[] = [];
  for (
    let parent = element.parentElement;
    parent && scope && parent !== scope && around.length < 4;
    parent = parent.parentElement
  )
    around.push(parent);
  return around;
}

/** Returns whether focus is on `<body>`, `<html>`, a disconnected node, or an unusable element. */
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

/**
 * Fallback focus order in a modal dialog when the focused control becomes
 * unavailable: the first invalid field, the dialog's autofocus control, the
 * first interactive body control, the primary footer button, any other footer
 * button, or the dialog container itself.
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
  if (!focusLost()) {
    // If focus remains on a shared fallback control (such as a Settings tab)
    // from a prior navigation and the user has not moved it, prefer the
    // remembered control for the new context.
    const active = document.activeElement;
    if (!navigated || !landed || active !== landed) return;
    for (const candidate of remembered.get(context) ?? []) {
      const target = resolve(candidate);
      if (target && target !== active) {
        target.focus();
        return;
      }
    }
    return;
  }
  requested = [];
  // Do not move focus on a blank-space click while the previous control
  // remains focusable, or on initial window load before any focus placement.
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
    // Re-focus a control that was only transiently disabled.
    ...(last?.taken ? [last.element] : []),
    ...(last?.identity ?? []),
    ...(last?.around ?? []).map(
      (container) => () =>
        container.isConnected
          ? Array.from(
              container.querySelectorAll<HTMLElement>(
                'button, input, select, textarea, a[href], [tabindex="0"]',
              ),
            ).find(focusable)
          : undefined,
    ),
  ];
  for (const candidate of candidates) {
    const target = resolve(candidate);
    if (target) {
      target.focus();
      return;
    }
  }
  const target = landing();
  target?.focus({ preventScroll: true });
  if (navigated) landed = target;
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
  if (!engaged) return;
  contextChanged = true;
  schedule();
}

/** Focus candidates saved when the keeper last unmounted. */
let parked: Candidate[] = [];
/** Focus candidates to restore on the next mount (after unlocking). */
let returnTo: Candidate[] = [];

/**
 * Preserves the shell's last focused control when the lock screen replaces the
 * shell, so unlocking restores focus to that control.
 */
export function holdFocusForReturn(): void {
  // The shell's keeper cleanup may run after the lock screen's layout effect;
  // `holding` instructs the cleanup to copy `parked` into `returnTo`.
  holding = true;
  returnTo = parked;
}
let holding = false;

/** Attaches document focus and mutation listeners; returns a cleanup function. */
export function mountFocusKeeper(
  root: HTMLElement = document.body,
): () => void {
  // Initial window mount is not a navigation; leave initial focus unchanged.
  contextChanged = false;
  engaged = false;
  parked = [];
  holding = false;
  if (returnTo.length) {
    const candidates = returnTo;
    returnTo = [];
    focusWhenRendered(() => {
      if (!focusLost()) return null;
      for (const candidate of candidates) {
        const target = resolve(candidate);
        if (target) return target;
      }
      return null;
      // Wait for the shell to reload the catalog after unlocking.
    }, 120);
  }
  const onFocusIn = (event: FocusEvent): void => {
    const target = event.target;
    if (!(target instanceof HTMLElement) || target === document.body) return;
    engaged = true;
    if (target !== landed) landed = null;
    // Clear pending programmatic focus requests when the user clicks.
    if (pressing) requested = [];
    const identity = focusIdentity(target);
    last = { element: target, identity, around: containersOf(target) };
    if (target.closest('.side, .main, .details') && !topDialog())
      inShell = last;
    // Record focus within `.main` and `.details` per context; sidebar focus
    // persists across navigation, and dialogs/popups do not.
    if (target.closest('.main, .details'))
      remembered.set(context, [...identity, sameLabelled(target)]);
  };
  let pressing = false;
  // Last control focused in the shell columns outside modal dialogs, restored
  // after unlocking.
  let inShell: Lost | null = null;
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
    parked = inShell
      ? [
          ...inShell.identity,
          sameLabelled(inShell.element, ['.side', '.main', '.details']),
        ]
      : [];
    if (holding) {
      returnTo = parked;
      holding = false;
    }
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
    landed = null;
    context = '';
    contextChanged = false;
    engaged = false;
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
