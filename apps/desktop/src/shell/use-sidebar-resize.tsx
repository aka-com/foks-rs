import { useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

const DEFAULT_WIDTH = 208;
const MIN_WIDTH = 150;
const MAX_WIDTH = 240;
const KEY = 'sidebarWidth';
const clamp = (width: number, max = MAX_WIDTH) =>
  Math.max(MIN_WIDTH, Math.min(max, width));

function storedWidth() {
  try {
    const value = Number(window.localStorage.getItem(KEY));
    return value > 0 && Number.isFinite(value) ? clamp(value) : DEFAULT_WIDTH;
  } catch {
    return DEFAULT_WIDTH;
  }
}

export function useSidebarResize(enabled: boolean) {
  const ref = useRef<HTMLElement>(null);
  const [host, setHost] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setHost(ref.current?.closest<HTMLElement>('.app') ?? null);
  }, []);
  const [width, setWidth] = useState(storedWidth);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ x: number; width: number; current: number } | null>(
    null,
  );
  const maxWidth = () => {
    const available = ref.current
      ?.closest('.window')
      ?.getBoundingClientRect().width;
    return available
      ? Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, available - 420))
      : MAX_WIDTH;
  };
  const save = (value: number) => {
    try {
      window.localStorage.setItem(KEY, String(value));
    } catch {
      /* Session only. */
    }
  };

  useLayoutEffect(() => {
    const frame = ref.current?.closest<HTMLElement>('.window');
    frame?.style.setProperty('--side-w-open', `${width}px`);
    return () => {
      frame?.style.removeProperty('--side-w-open');
    };
  }, [width]);

  useLayoutEffect(() => {
    if (!dragging) return;
    const previous = document.body.style.cursor;
    const selection = document.body.style.userSelect;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.cursor = previous;
      document.body.style.userSelect = selection;
    };
  }, [dragging]);

  const finish = () => {
    if (drag.current) save(drag.current.current);
    drag.current = null;
    setDragging(false);
  };

  return {
    ref,
    dragging,
    handle:
      enabled && host
        ? createPortal(
            <div
              className={
                dragging ? 'sidebar-resizer is-resizing' : 'sidebar-resizer'
              }
              role="separator"
              aria-label="Resize sidebar"
              aria-orientation="vertical"
              aria-valuemin={MIN_WIDTH}
              aria-valuemax={MAX_WIDTH}
              aria-valuenow={width}
              tabIndex={0}
              onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.currentTarget.focus();
                event.currentTarget.setPointerCapture(event.pointerId);
                drag.current = { x: event.clientX, width, current: width };
                setDragging(true);
              }}
              onPointerMove={(event) => {
                if (!drag.current) return;
                const next = clamp(
                  drag.current.width + event.clientX - drag.current.x,
                  maxWidth(),
                );
                drag.current.current = next;
                setWidth(next);
              }}
              onPointerUp={finish}
              onPointerCancel={finish}
              onLostPointerCapture={finish}
              onDoubleClick={() => {
                setWidth(DEFAULT_WIDTH);
                save(DEFAULT_WIDTH);
              }}
              onKeyDown={(event) => {
                let next: number;
                if (event.key === 'ArrowLeft') next = width - 8;
                else if (event.key === 'ArrowRight') next = width + 8;
                else if (event.key === 'Home') next = MIN_WIDTH;
                else if (event.key === 'End') next = maxWidth();
                else return;
                event.preventDefault();
                next = clamp(next, maxWidth());
                setWidth(next);
                save(next);
              }}
            />,
            host,
          )
        : null,
  };
}
