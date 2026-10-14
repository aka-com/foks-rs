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
/** How many shown messages the controller keeps for the Recent list. */
export const TOAST_HISTORY_LENGTH = 20;

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

/**
 * One message the controller showed, as the Recent list draws it: the text
 * and when it was shown. Actions are not kept; the list only reports.
 */
export interface ToastRecord {
  id: number;
  message: string;
  tone: 'info' | 'warning';
  /** Milliseconds since the epoch. */
  shownAt: number;
}

const NO_HISTORY: readonly ToastRecord[] = [];

/** A message is a warning when its caller says so or it starts with ⚠. */
function toastTone(
  request: ToastOptions & { message: string },
): ToastRecord['tone'] {
  return request.tone === 'warning' ||
    (request.tone === undefined && request.message.trimStart().startsWith('⚠'))
    ? 'warning'
    : 'info';
}

/** Explicit notification channel owned by one mounted ToastProvider. */
export class ToastController {
  private listener: ToastListener | null = null;
  private pending: ToastRequest[] = [];
  private nextId = 1;
  private history: readonly ToastRecord[] = NO_HISTORY;
  /** The history with each entry's dedupe key, which the list does not show. */
  private keyed: readonly { key: string; record: ToastRecord }[] = [];
  private historyListeners = new Set<() => void>();

  constructor(private readonly now: () => number = Date.now) {}

  show(message: string, options: ToastOptions = {}): number {
    const request = { id: this.nextId++, message, ...options };
    this.record(request);
    if (this.listener) this.listener(request);
    else {
      this.pending.push(request);
      this.pending = this.pending.slice(-MAX_VISIBLE_TOASTS);
    }
    return request.id;
  }

  /**
   * The last `TOAST_HISTORY_LENGTH` messages, oldest first. Only the message
   * text is kept, in memory, for as long as this controller lives; callers
   * clear it when the app locks. A message that replaces a shown one (the
   * same `dedupeKey`, or the same text) replaces its entry as well, so a
   * progress message and its outcome take one row.
   */
  getHistory = (): readonly ToastRecord[] => this.history;

  subscribeHistory = (listener: () => void): (() => void) => {
    this.historyListeners.add(listener);
    return () => {
      this.historyListeners.delete(listener);
    };
  };

  clearHistory(): void {
    if (!this.history.length) return;
    this.history = NO_HISTORY;
    this.keyed = [];
    this.publishHistory();
  }

  private record(request: ToastRequest): void {
    const key = request.dedupeKey ?? request.message;
    const record: ToastRecord = {
      id: request.id,
      message: request.message,
      tone: toastTone(request),
      shownAt: this.now(),
    };
    this.keyed = [
      ...this.keyed.filter((entry) => entry.key !== key),
      { key, record },
    ].slice(-TOAST_HISTORY_LENGTH);
    this.history = this.keyed.map((entry) => entry.record);
    this.publishHistory();
  }

  private publishHistory(): void {
    for (const listener of this.historyListeners) listener();
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

export function useOptionalToast(): ToastController | null {
  return useContext(ToastContext);
}

function ToastItem({
  entry,
  dismiss,
}: {
  entry: ToastEntry;
  dismiss: (id: number) => void;
}): ReactNode {
  const [visible, setVisible] = useState(false);
  const warning = toastTone(entry) === 'warning';

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
      role={warning ? 'alert' : 'status'}
      aria-atomic="true"
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
    <div id="toasts" className="toasts">
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
