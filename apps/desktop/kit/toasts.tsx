import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useState,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

const DEFAULT_DURATION_MS = 2_600;
const EXIT_DURATION_MS = 300;
const MAX_VISIBLE_TOASTS = 5;

export interface ToastAction {
  label: string;
  onAction: () => void;
}

export interface ToastOptions {
  action?: ToastAction;
  dedupeKey?: string;
  durationMs?: number;
  tone?: 'info' | 'warning';
}

interface ToastRequest extends ToastOptions {
  id: number;
  message: string;
}

type ToastListener = (request: ToastRequest) => void;

/** Explicit notification channel owned by one mounted ToastProvider. */
export class ToastController {
  private listener: ToastListener | null = null;
  private pending: ToastRequest[] = [];
  private nextId = 1;

  show(message: string, options: ToastOptions = {}): number {
    const request = { id: this.nextId++, message, ...options };
    if (this.listener) this.listener(request);
    else {
      this.pending.push(request);
      this.pending = this.pending.slice(-MAX_VISIBLE_TOASTS);
    }
    return request.id;
  }

  connect(listener: ToastListener): () => void {
    if (this.listener) {
      throw new Error('A ToastController cannot serve multiple providers');
    }
    this.listener = listener;
    const queued = this.pending;
    this.pending = [];
    for (const request of queued) listener(request);
    return () => {
      if (this.listener === listener) this.listener = null;
    };
  }
}

interface ToastEntry extends ToastRequest {
  revision: number;
}

const ToastContext = createContext<ToastController | null>(null);

export function useToast(): ToastController {
  const controller = useContext(ToastContext);
  if (!controller)
    throw new Error('useToast must be used inside ToastProvider');
  return controller;
}

function ToastItem({
  entry,
  dismiss,
}: {
  entry: ToastEntry;
  dismiss: (id: number) => void;
}): ReactNode {
  const [visible, setVisible] = useState(false);
  const warning =
    entry.tone === 'warning' ||
    (entry.tone === undefined && entry.message.trimStart().startsWith('⚠'));

  useEffect(() => {
    const animationFrame = window.requestAnimationFrame(() => setVisible(true));
    // Actionable toasts remain open indefinitely by default until dismissed or timed out by the caller.
    if (entry.action && entry.durationMs === undefined) {
      return () => window.cancelAnimationFrame(animationFrame);
    }
    const duration = Math.max(0, entry.durationMs ?? DEFAULT_DURATION_MS);
    const hideTimer = window.setTimeout(() => setVisible(false), duration);
    const removeTimer = window.setTimeout(
      () => dismiss(entry.id),
      duration + EXIT_DURATION_MS,
    );
    return () => {
      window.cancelAnimationFrame(animationFrame);
      window.clearTimeout(hideTimer);
      window.clearTimeout(removeTimer);
    };
  }, [dismiss, entry.action, entry.durationMs, entry.id, entry.revision]);

  return (
    <div
      className={`toast toast-owned${visible ? ' show' : ''}`}
      data-toast-id={entry.id}
      data-toast-tone={warning ? 'warning' : 'info'}
      role={warning ? 'alert' : undefined}
      aria-live={warning ? 'assertive' : undefined}
    >
      <span className="toast-message">{entry.message}</span>
      {entry.action ? (
        <button
          type="button"
          className="toast-action"
          onClick={() => {
            try {
              entry.action?.onAction();
            } finally {
              dismiss(entry.id);
            }
          }}
        >
          {entry.action.label}
        </button>
      ) : null}
      <button
        type="button"
        className="toast-dismiss"
        aria-label="Dismiss notification"
        data-toast-dismiss
        onClick={() => dismiss(entry.id)}
      >
        ×
      </button>
    </div>
  );
}

export function ToastProvider({
  children,
  controller,
  portalRoot,
}: {
  children: ReactNode;
  controller: ToastController;
  /** Overlay host. Sheets portal here too; toasts must share that layer. */
  portalRoot?: HTMLElement | null;
}): ReactNode {
  const [entries, setEntries] = useState<ToastEntry[]>([]);
  const dismiss = useCallback((id: number) => {
    setEntries((current) => current.filter((entry) => entry.id !== id));
  }, []);

  useLayoutEffect(
    () =>
      controller.connect((request) => {
        setEntries((current) => {
          const dedupeKey = request.dedupeKey ?? request.message;
          const duplicate = current.findIndex(
            (entry) => (entry.dedupeKey ?? entry.message) === dedupeKey,
          );
          if (duplicate >= 0) {
            const next = [...current];
            next[duplicate] = {
              ...request,
              id: current[duplicate].id,
              revision: current[duplicate].revision + 1,
            };
            return next;
          }
          return [...current, { ...request, revision: 0 }].slice(
            -MAX_VISIBLE_TOASTS,
          );
        });
      }),
    [controller],
  );

  const host = (
    <div
      id="toasts"
      className="toasts"
      role="status"
      aria-live="polite"
      aria-atomic="false"
    >
      {entries.map((entry) => (
        <ToastItem key={entry.id} entry={entry} dismiss={dismiss} />
      ))}
    </div>
  );

  return (
    <ToastContext.Provider value={controller}>
      {children}
      {portalRoot ? createPortal(host, portalRoot) : host}
    </ToastContext.Provider>
  );
}
