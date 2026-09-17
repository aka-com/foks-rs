import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { createPortal } from 'react-dom';

import { placeAnchoredMenu, type MenuAlign } from './menu-position';

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), ' +
  'textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

interface IsolationState {
  count: number;
  inert: boolean;
  ariaHidden: string | null;
}

const isolation = new WeakMap<HTMLElement, IsolationState>();

function isolate(element: HTMLElement): () => void {
  let state = isolation.get(element);
  if (!state) {
    state = {
      count: 0,
      inert: element.inert === true,
      ariaHidden: element.getAttribute('aria-hidden'),
    };
    isolation.set(element, state);
  }
  state.count += 1;
  element.inert = true;
  element.setAttribute('aria-hidden', 'true');
  return () => {
    const current = isolation.get(element);
    if (!current) return;
    current.count -= 1;
    if (current.count > 0) return;
    element.inert = current.inert;
    if (current.ariaHidden === null) element.removeAttribute('aria-hidden');
    else element.setAttribute('aria-hidden', current.ariaHidden);
    isolation.delete(element);
  };
}

interface OverlayEnvironment {
  backgroundRef: RefObject<HTMLElement | null>;
  portalRoot: HTMLElement;
  dialogs: Set<HTMLElement>;
}

/**
 * Tracks mounted dialogs across overlay providers. Individual providers track
 * their dialogs separately for focus restoration.
 */
const mountedDialogs = new Set<HTMLElement>();

/**
 * Returns whether any modal dialog is open. Shell-level input handlers use
 * this to suspend shortcuts and gestures while a modal is active.
 */
export function anyDialogOpen(): boolean {
  return mountedDialogs.size > 0;
}

const OverlayContext = createContext<OverlayEnvironment | null>(null);

export function OverlayProvider({
  backgroundRef,
  portalRoot,
  children,
}: {
  backgroundRef: RefObject<HTMLElement | null>;
  portalRoot: HTMLElement;
  children: ReactNode;
}): ReactNode {
  const environment = useRef<OverlayEnvironment>({
    backgroundRef,
    portalRoot,
    dialogs: new Set<HTMLElement>(),
  }).current;
  environment.backgroundRef = backgroundRef;
  environment.portalRoot = portalRoot;
  return (
    <OverlayContext.Provider value={environment}>
      {children}
    </OverlayContext.Provider>
  );
}

export function useHasOverlayProvider(): boolean {
  return useContext(OverlayContext) !== null;
}

function useOverlayEnvironment(): OverlayEnvironment {
  const environment = useContext(OverlayContext);
  if (!environment) {
    throw new Error(
      'Overlay primitives must be mounted inside OverlayProvider',
    );
  }
  return environment;
}

function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
  ).filter(
    (element) =>
      !element.inert && element.getAttribute('aria-hidden') !== 'true',
  );
}

function canRestoreFocus(
  element: HTMLElement,
  dialogs: Set<HTMLElement>,
): boolean {
  if (!element.isConnected) return false;
  for (
    let current: HTMLElement | null = element;
    current;
    current = current.parentElement
  ) {
    if (current.inert || current.getAttribute('aria-hidden') === 'true')
      return false;
  }
  return (
    dialogs.size === 0 ||
    Array.from(dialogs).some(
      (dialog) => !dialog.inert && dialog.contains(element),
    )
  );
}

export interface DialogProps extends Omit<
  HTMLAttributes<HTMLDivElement>,
  'role'
> {
  children: ReactNode;
  className?: string;
  role?: 'dialog' | 'alertdialog';
  titleId?: string;
}

export function Dialog({
  children,
  className = '',
  role = 'dialog',
  titleId,
  ...attributes
}: DialogProps): ReactNode {
  const environment = useOverlayEnvironment();
  const { dialogs } = environment;
  const dialogRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const opener =
      document.activeElement instanceof HTMLElement &&
      document.activeElement !== document.body
        ? document.activeElement
        : null;
    const releases = [
      ...(environment.backgroundRef.current
        ? [isolate(environment.backgroundRef.current)]
        : []),
      ...Array.from(dialogs, isolate),
    ];
    dialogs.add(dialog);
    mountedDialogs.add(dialog);
    const initial =
      dialog.querySelector<HTMLElement>(
        '[data-dialog-autofocus="true"], [data-sheet-autofocus="true"]',
      ) ??
      focusables(dialog)[0] ??
      dialog;
    initial.focus();
    return () => {
      dialogs.delete(dialog);
      mountedDialogs.delete(dialog);
      for (const release of releases.reverse()) release();
      queueMicrotask(() => {
        if (!dialog.isConnected && opener && canRestoreFocus(opener, dialogs))
          opener.focus();
      });
    };
  }, [dialogs, environment]);

  return (
    <div
      {...attributes}
      ref={dialogRef}
      className={className}
      role={role}
      aria-modal="true"
      aria-labelledby={titleId}
      tabIndex={-1}
      onKeyDown={(event) => {
        attributes.onKeyDown?.(event);
        if (event.defaultPrevented || event.key !== 'Tab') return;
        const candidates = focusables(event.currentTarget);
        if (!candidates.length) {
          event.preventDefault();
          event.currentTarget.focus();
          return;
        }
        const first = candidates[0];
        const last = candidates[candidates.length - 1];
        const inside = event.currentTarget.contains(document.activeElement);
        if (event.shiftKey && (!inside || document.activeElement === first)) {
          event.preventDefault();
          last.focus();
        } else if (
          !event.shiftKey &&
          (!inside || document.activeElement === last)
        ) {
          event.preventDefault();
          first.focus();
        }
      }}
    >
      {children}
    </div>
  );
}

export function DismissibleDialog({
  onDismiss,
  dismissible = true,
  onMouseDown,
  onKeyDown,
  ...props
}: DialogProps & {
  onDismiss: () => void;
  dismissible?: boolean;
}): ReactNode {
  const { portalRoot } = useOverlayEnvironment();
  return createPortal(
    <Dialog
      {...props}
      onMouseDown={(event) => {
        onMouseDown?.(event);
        if (
          !dismissible ||
          event.defaultPrevented ||
          event.target !== event.currentTarget
        )
          return;
        onDismiss();
      }}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (!dismissible || event.defaultPrevented || event.key !== 'Escape')
          return;
        event.preventDefault();
        onDismiss();
      }}
    />,
    portalRoot,
  );
}

function moveFocus(items: HTMLElement[], key: string): void {
  if (!items.length) return;
  const current = items.indexOf(document.activeElement as HTMLElement);
  const next =
    key === 'Home'
      ? 0
      : key === 'End'
        ? items.length - 1
        : current < 0
          ? key === 'ArrowUp'
            ? items.length - 1
            : 0
          : Math.min(
              Math.max(current + (key === 'ArrowUp' ? -1 : 1), 0),
              items.length - 1,
            );
  items[next].focus();
}

function moveFocusFromAnchor(anchor: HTMLElement, backwards: boolean): void {
  const scope = anchor.closest<HTMLElement>(
    '[role="dialog"], [role="alertdialog"]',
  );
  const candidates = focusables(scope ?? document.body);
  const current = candidates.indexOf(anchor);
  if (current < 0 || !candidates.length) {
    anchor.focus();
    return;
  }
  const offset = backwards ? -1 : 1;
  candidates[
    (current + offset + candidates.length) % candidates.length
  ].focus();
}

function RovingCollection({
  kind,
  children,
  anchorRef,
  onClose,
  initialFocus = 'selected',
  ...attributes
}: {
  kind: 'menu' | 'listbox';
  children: ReactNode;
  anchorRef?: RefObject<HTMLElement | null>;
  onClose?: () => void;
  initialFocus?: 'selected' | 'first' | 'last' | 'none';
} & HTMLAttributes<HTMLDivElement>): ReactNode {
  const { dialogs } = useOverlayEnvironment();
  const collectionRef = useRef<HTMLDivElement>(null);
  // A menu keeps the items that do not apply in its keyboard order: they are
  // marked `aria-disabled` and do nothing, but arrowing onto one is how their
  // reason gets read out. A listbox still skips its unavailable options, and a
  // natively `disabled` control cannot hold focus in either.
  const itemSelector =
    kind === 'menu'
      ? '[role="menuitem"]:not(button), button:not([disabled])'
      : '[role="option"]:not([aria-disabled="true"])';

  const [focusTarget, setFocusTarget] = useState<HTMLElement | null>(null);

  useLayoutEffect(() => {
    if (initialFocus === 'none') return;
    const collection = collectionRef.current;
    if (!collection) return;
    const items = Array.from(
      collection.querySelectorAll<HTMLElement>(itemSelector),
    );
    for (const item of items) {
      if (kind === 'menu' && !item.hasAttribute('role'))
        item.setAttribute('role', 'menuitem');
      item.tabIndex = -1;
    }
    const selected =
      initialFocus === 'selected'
        ? items.find((item) => item.getAttribute('aria-selected') === 'true')
        : null;
    const target =
      initialFocus === 'last'
        ? items[items.length - 1]
        : (selected ?? items[0]);
    setFocusTarget(target ?? null);
  }, [initialFocus, itemSelector, kind]);

  // Focus after layout and positioning to prevent flicker.
  // The menu must be visible before its focus target can receive focus.
  useEffect(() => {
    if (focusTarget?.isConnected) focusTarget.focus();
  }, [focusTarget]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    attributes.onKeyDown?.(event);
    if (event.defaultPrevented) return;
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      moveFocus(
        Array.from(
          event.currentTarget.querySelectorAll<HTMLElement>(itemSelector),
        ),
        event.key,
      );
      return;
    }
    if (event.key === 'Tab' && onClose) {
      const anchor = anchorRef?.current;
      if (!anchor) {
        onClose();
        return;
      }
      event.preventDefault();
      onClose();
      moveFocusFromAnchor(anchor, event.shiftKey);
    }
  };

  const handleClick = (event: ReactMouseEvent<HTMLDivElement>): void => {
    attributes.onClick?.(event);
    if (event.defaultPrevented || !anchorRef?.current) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target?.closest(itemSelector)) return;
    const anchor = anchorRef.current;
    const collection = event.currentTarget;
    queueMicrotask(() => {
      if (!collection.isConnected && canRestoreFocus(anchor, dialogs))
        anchor.focus();
    });
  };

  return (
    <div
      {...attributes}
      ref={collectionRef}
      role={kind}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      {children}
    </div>
  );
}

export function Menu(
  props: Omit<Parameters<typeof RovingCollection>[0], 'kind'>,
): ReactNode {
  return <RovingCollection {...props} kind="menu" />;
}

export function Listbox(
  props: Omit<Parameters<typeof RovingCollection>[0], 'kind'>,
): ReactNode {
  return <RovingCollection {...props} kind="listbox" />;
}

export function Popover({
  anchorRef,
  children,
  className = '',
  align = 'end',
  gap = 4,
  matchAnchorWidth = false,
  minWidth,
  onClose,
}: {
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  className?: string;
  align?: MenuAlign;
  gap?: number;
  matchAnchorWidth?: boolean;
  minWidth?: number;
  onClose: () => void;
}): ReactNode {
  const { dialogs, portalRoot } = useOverlayEnvironment();
  const popoverRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    const popover = popoverRef.current;
    if (!anchor || !popover) return;
    const position = () => {
      if (matchAnchorWidth || minWidth !== undefined) {
        const width = Math.max(
          matchAnchorWidth ? anchor.getBoundingClientRect().width : 0,
          minWidth ?? 0,
        );
        popover.style.width = `${width}px`;
      }
      placeAnchoredMenu(popover, anchor, align, gap);
    };
    const pointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (anchor.contains(target) || popover.contains(target)) return;
      onCloseRef.current();
    };
    const keyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      event.stopPropagation();
      onCloseRef.current();
      if (canRestoreFocus(anchor, dialogs)) anchor.focus();
    };
    position();
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    document.addEventListener('pointerdown', pointerDown, true);
    document.addEventListener('keydown', keyDown, true);
    return () => {
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
      document.removeEventListener('pointerdown', pointerDown, true);
      document.removeEventListener('keydown', keyDown, true);
    };
  }, [align, anchorRef, dialogs, gap, matchAnchorWidth, minWidth]);

  return createPortal(
    <div ref={popoverRef} className={className}>
      {children}
    </div>,
    portalRoot,
  );
}

export function ContextMenu({
  point,
  children,
  className = '',
  onClose,
}: {
  point: { x: number; y: number };
  children: ReactNode;
  className?: string;
  onClose: () => void;
}): ReactNode {
  const { portalRoot } = useOverlayEnvironment();
  const contextRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useLayoutEffect(() => {
    const context = contextRef.current;
    if (!context) return;
    const position = () => {
      const inset = 8;
      const box = context.getBoundingClientRect();
      context.style.left = `${Math.min(
        Math.max(inset, point.x),
        Math.max(inset, window.innerWidth - box.width - inset),
      )}px`;
      context.style.top = `${Math.min(
        Math.max(inset, point.y),
        Math.max(inset, window.innerHeight - box.height - inset),
      )}px`;
      context.style.visibility = 'visible';
    };
    const pointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !context.contains(event.target))
        onCloseRef.current();
    };
    const keyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      event.stopPropagation();
      onCloseRef.current();
    };
    position();
    window.addEventListener('resize', position);
    document.addEventListener('pointerdown', pointerDown, true);
    document.addEventListener('keydown', keyDown, true);
    return () => {
      window.removeEventListener('resize', position);
      document.removeEventListener('pointerdown', pointerDown, true);
      document.removeEventListener('keydown', keyDown, true);
    };
  }, [point.x, point.y]);

  return createPortal(
    <div ref={contextRef} className={className}>
      {children}
    </div>,
    portalRoot,
  );
}
