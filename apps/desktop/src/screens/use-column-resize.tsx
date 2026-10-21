import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactNode, RefObject } from 'react';

const clamp = (value: number, min: number, max: number) =>
  Math.max(min, Math.min(max, value));

interface ColumnResizeOptions {
  storageKey: string;
  cssVariable: `--${string}`;
  className: string;
  label: string;
  defaultWidth: number;
  compactWidth: number;
  compactAt?: number;
  minWidth?: number;
  maxWidth?: number;
  reservedWidth?: number;
}

export interface ColumnResize<T extends HTMLElement> {
  ref: RefObject<T | null>;
  id: string;
  style: CSSProperties;
  handle: ReactNode;
}

/**
 * Pointer, keyboard and persistence for a resizable column inside the panel
 * (Chat's channel column), measured against its own frame and returned as a
 * style. The window's sidebar has its own hook (`shell/use-sidebar-resize`):
 * it is measured against the window and its divider is portaled into `.app`.
 */
export function useColumnResize<T extends HTMLElement = HTMLElement>({
  storageKey,
  cssVariable,
  className,
  label,
  defaultWidth,
  compactWidth,
  compactAt = 900,
  minWidth = 180,
  maxWidth = 420,
  reservedWidth = 320,
}: ColumnResizeOptions): ColumnResize<T> {
  const readWidth = (): number | null => {
    try {
      const value = Number(window.localStorage.getItem(storageKey));
      return Number.isFinite(value) && value > 0
        ? clamp(value, minWidth, maxWidth)
        : null;
    } catch {
      return null;
    }
  };
  const ref = useRef<T>(null);
  const id = useId();
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
    const frame = ref.current;
    if (!frame) return;
    const measure = () => setAvailable(frame.getBoundingClientRect().width);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(frame);
    return () => observer.disconnect();
  }, []);

  const max =
    available > 0
      ? Math.min(
          maxWidth,
          available - Math.min(reservedWidth, available * 0.55),
        )
      : maxWidth;
  const min = Math.min(minWidth, max);
  const responsiveDefault =
    available > 0 && available <= compactAt ? compactWidth : defaultWidth;
  const width = clamp(preferred ?? responsiveDefault, min, max);
  const save = (value: number | null) => {
    try {
      if (value === null) window.localStorage.removeItem(storageKey);
      else window.localStorage.setItem(storageKey, String(value));
    } catch {
      // Storage can be unavailable; resizing still works for this mount.
    }
  };
  const finish = (commit: boolean) => {
    const active = drag.current;
    if (!active) return;
    drag.current = null;
    if (commit && active.current !== active.width) save(active.current);
    else setPreferred(active.previous);
    setDragging(false);
  };

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

  return {
    ref,
    id,
    style: { [cssVariable]: `${width}px` },
    handle: (
      <div
        className={`${className}${dragging ? ' is-resizing' : ''}`}
        role="separator"
        aria-label={`Resize ${label} sidebar`}
        aria-controls={id}
        aria-orientation="vertical"
        aria-valuemin={Math.round(min)}
        aria-valuemax={Math.round(max)}
        aria-valuenow={Math.round(width)}
        aria-valuetext={`${Math.round(width)} pixels`}
        title="Drag to resize; double-click to reset"
        // Out of the Tab loop, like the sidebar's divider: VoiceOver and a
        // press still focus it, and the arrows then resize.
        tabIndex={-1}
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
    ),
  };
}
