import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, RefObject } from 'react';
import { createPortal } from 'react-dom';

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
  type DeviceStorageKey,
} from '../platform/device-storage';

/**
 * One resizable pane. Widths are CSS pixels. The stored width is a
 * device-local display preference, intentionally absent from Settings.
 */
export interface PaneSize {
  storageKey: DeviceStorageKey;
  /** Receives the current width, for example `264px`. */
  cssVariable: `--${string}`;
  /** Separator class; `is-resizing` is added during a drag. */
  className: string;
  /** Accessible name of the separator. */
  label: string;
  min: number;
  max: number;
  /** Width without a stored preference in a frame wider than `compactAt`. */
  default: number;
  /** Width without a stored preference in a frame at most `compactAt` wide. */
  compact: number;
  compactAt: number;
  /** Frame width kept for the rest of the layout. */
  reserved: number;
  /** Largest share of the frame width that `reserved` may claim. */
  reservedShare: number;
  /**
   * Whether `min` yields to a frame too narrow for it. When false, the pane
   * keeps `min` and the rest of the layout gets less than `reserved`.
   */
  shrinks: boolean;
  /**
   * The element whose width bounds the pane and that receives `cssVariable`,
   * found from the element the hook's ref is attached to. Without it, that
   * element is the frame and applies the returned `style` itself.
   */
  frame?: (own: HTMLElement) => HTMLElement | null;
  /** The element the separator is rendered into, instead of in place. */
  portal?: (own: HTMLElement) => HTMLElement | null;
}

/** The navigation rail, the Files folder tree and the Chat column. */
export const PANE_SIZES = {
  rail: {
    storageKey: STORAGE_KEYS.sidebarWidth,
    cssVariable: '--side-w-open',
    className: 'sidebar-resizer',
    label: 'Resize sidebar',
    min: 150,
    max: 240,
    default: 150,
    compact: 150,
    compactAt: 900,
    reserved: 420,
    reservedShare: 1,
    shrinks: false,
    // The rail is a grid item of `.app`; its width variable is read by the
    // whole `.window`, and its separator sits on the `.app` grid edge.
    frame: (own) => own.closest<HTMLElement>('.window'),
    portal: (own) => own.closest<HTMLElement>('.app'),
  },
  files: {
    storageKey: STORAGE_KEYS.filesSidebarWidth,
    cssVariable: '--files-tree-w',
    className: 'files-sidebar-resizer',
    label: 'Resize Files sidebar',
    min: 180,
    max: 420,
    default: 232,
    compact: 220,
    compactAt: 900,
    reserved: 320,
    reservedShare: 0.55,
    shrinks: true,
  },
  chat: {
    storageKey: STORAGE_KEYS.chatSidebarWidth,
    cssVariable: '--chat-inbox-w',
    className: 'chat-sidebar-resizer',
    label: 'Resize Chat sidebar',
    min: 180,
    max: 420,
    default: 264,
    compact: 220,
    compactAt: 900,
    reserved: 320,
    reservedShare: 0.55,
    shrinks: true,
  },
} as const satisfies Record<string, PaneSize>;

export interface PaneResize<T extends HTMLElement> {
  /** Attach to the pane's frame, or to the pane when `frame` names another. */
  ref: RefObject<T | null>;
  /** Give this id to the resized pane; the separator's `aria-controls`. */
  id: string;
  style: CSSProperties;
  dragging: boolean;
  /** The separator, or null while disabled. */
  handle: ReactNode;
}

const clamp = (value: number, min: number, max: number) =>
  Math.max(min, Math.min(max, value));

/**
 * Pointer, keyboard and persistence behavior for a pane's separator. A drag
 * commits on release; Escape, pointer cancellation and lost capture restore
 * the width the drag started from. The stored preference is kept when a
 * narrow frame bounds the displayed width, and double-click clears it.
 */
export function usePaneResize<T extends HTMLElement = HTMLElement>(
  pane: PaneSize,
  enabled = true,
): PaneResize<T> {
  const { storageKey, cssVariable, frame, portal } = pane;
  const readWidth = (): number | null => {
    const value = Number(readStorage(storageKey));
    return Number.isFinite(value) && value > 0
      ? clamp(value, pane.min, pane.max)
      : null;
  };
  const ref = useRef<T>(null);
  const id = useId();
  const [host, setHost] = useState<HTMLElement | null>(null);
  const [preferred, setPreferred] = useState(readWidth);
  const [available, setAvailable] = useState(0);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{
    pointer: number;
    x: number;
    width: number;
    previous: number | null;
    current: number;
  } | null>(null);

  useLayoutEffect(() => {
    const own = ref.current;
    if (!own) return;
    setHost(portal ? portal(own) : null);
    const target = frame ? frame(own) : own;
    if (!target) return;
    const measure = () => setAvailable(target.getBoundingClientRect().width);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(target);
    return () => observer.disconnect();
  }, [frame, portal]);

  const limit =
    available > 0
      ? Math.min(
          pane.max,
          available - Math.min(pane.reserved, available * pane.reservedShare),
        )
      : pane.max;
  const max = pane.shrinks ? limit : Math.max(pane.min, limit);
  const min = Math.min(pane.min, max);
  const responsiveDefault =
    available > 0 && available <= pane.compactAt ? pane.compact : pane.default;
  const width = clamp(preferred ?? responsiveDefault, min, max);
  // Storage can be unavailable; resizing still works for this mount.
  const save = (value: number | null) =>
    writeStorage(storageKey, value === null ? null : String(value));
  const finish = (commit: boolean) => {
    const active = drag.current;
    if (!active) return;
    drag.current = null;
    if (commit && active.current !== active.width) save(active.current);
    else setPreferred(active.previous);
    setDragging(false);
  };

  useLayoutEffect(() => {
    const own = ref.current;
    const target = own && frame ? frame(own) : null;
    target?.style.setProperty(cssVariable, `${width}px`);
    return () => {
      target?.style.removeProperty(cssVariable);
    };
  }, [frame, cssVariable, width]);

  // A separator removed mid-drag receives no further pointer events.
  useLayoutEffect(() => {
    const active = drag.current;
    if (enabled || !active) return;
    drag.current = null;
    setPreferred(active.previous);
    setDragging(false);
  }, [enabled]);

  useLayoutEffect(() => {
    if (!dragging) return;
    const { cursor, userSelect } = document.body.style;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.cursor = cursor;
      document.body.style.userSelect = userSelect;
    };
  }, [dragging]);

  const separator = (
    <div
      className={`${pane.className}${dragging ? ' is-resizing' : ''}`}
      role="separator"
      aria-label={pane.label}
      aria-controls={id}
      aria-orientation="vertical"
      aria-valuemin={Math.round(min)}
      aria-valuemax={Math.round(max)}
      aria-valuenow={Math.round(width)}
      aria-valuetext={`${Math.round(width)} pixels`}
      title="Drag to resize; double-click to reset"
      tabIndex={0}
      onPointerDown={(event) => {
        if (event.button !== 0 || drag.current) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = {
          pointer: event.pointerId,
          x: event.clientX,
          width,
          previous: preferred,
          current: width,
        };
        setDragging(true);
      }}
      onPointerMove={(event) => {
        const active = drag.current;
        if (!active || active.pointer !== event.pointerId) return;
        const next = clamp(active.width + event.clientX - active.x, min, max);
        active.current = next;
        setPreferred(next);
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer === event.pointerId) {
          finish(true);
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
      onPointerCancel={(event) => {
        if (drag.current?.pointer === event.pointerId) finish(false);
      }}
      onLostPointerCapture={(event) => {
        if (drag.current?.pointer === event.pointerId) finish(false);
      }}
      onDoubleClick={() => {
        finish(false);
        setPreferred(null);
        save(null);
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape' && drag.current) {
          event.preventDefault();
          const pointer = drag.current.pointer;
          finish(false);
          event.currentTarget.releasePointerCapture(pointer);
          return;
        }
        if (drag.current) return;
        let next: number;
        if (event.key === 'ArrowLeft') next = width - 8;
        else if (event.key === 'ArrowRight') next = width + 8;
        else if (event.key === 'Home') next = min;
        else if (event.key === 'End') next = max;
        else return;
        event.preventDefault();
        next = clamp(next, min, max);
        setPreferred(next);
        save(next);
      }}
    />
  );

  return {
    ref,
    id,
    style: frame ? {} : { [cssVariable]: `${width}px` },
    dragging,
    handle: !enabled
      ? null
      : portal
        ? host && createPortal(separator, host)
        : separator,
  };
}
