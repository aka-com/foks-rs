import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';

/** The shortest time an information toast stays on screen. */
export const MIN_INFO_DURATION_MS = 4_000;
/** The shortest time a warning stays on screen. */
export const MIN_WARNING_DURATION_MS = 8_000;
/** The longest time a toast without a caller-given duration stays. */
export const MAX_DURATION_MS = 20_000;
/**
 * Reading time per character: about 1,000 characters a minute, or 200
 * words of five letters and a space. One further second is allowed for
 * noticing the toast.
 */
const READING_MS_PER_CHARACTER = 60;
const NOTICE_MS = 1_000;
/** A toast released by the pointer or focus stays at least this long. */
const RESUME_MIN_MS = 1_000;
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

function toastTone(request: ToastOptions): ToastRecord['tone'] {
  return request.tone === 'warning' ? 'warning' : 'info';
}

/**
 * How long a toast without a caller-given `durationMs` stays on screen: its
 * reading time, at least {@link MIN_INFO_DURATION_MS} or, for a warning,
 * {@link MIN_WARNING_DURATION_MS}, and at most {@link MAX_DURATION_MS}. Time
 * while the toast is hovered, holds focus or the window is hidden does not
 * count.
 */
export function toastDurationMs(
  message: string,
  tone: ToastRecord['tone'],
): number {
  const floor =
    tone === 'warning' ? MIN_WARNING_DURATION_MS : MIN_INFO_DURATION_MS;
  const reading = NOTICE_MS + message.length * READING_MS_PER_CHARACTER;
  return Math.min(MAX_DURATION_MS, Math.max(floor, reading));
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

/** Whether the window is hidden, as `document.hidden` reports it. */
function useDocumentHidden(): boolean {
  const [hidden, setHidden] = useState(
    () => typeof document !== 'undefined' && document.hidden,
  );
  useEffect(() => {
    const update = () => setHidden(document.hidden);
    update();
    document.addEventListener('visibilitychange', update);
    return () => document.removeEventListener('visibilitychange', update);
  }, []);
  return hidden;
}

function ToastItem({
  entry,
  dismiss,
  hidden,
}: {
  entry: ToastEntry;
  dismiss: (id: number) => void;
  /** Whether the window is hidden, which holds the toast like a hover. */
  hidden: boolean;
}): ReactNode {
  const [visible, setVisible] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  // An actionable toast stays until it is dismissed unless its caller gives a duration.
  const duration =
    entry.action && entry.durationMs === undefined
      ? null
      : Math.max(
          0,
          entry.durationMs ?? toastDurationMs(entry.message, toastTone(entry)),
        );
  const held = hovered || focused || hidden;
  // The time left on screen, which counts down only while the toast is not
  // held and starts again when a deduplicated message replaces the text.
  const remaining = useRef({ revision: entry.revision, ms: duration });

  useEffect(() => {
    const animationFrame = window.requestAnimationFrame(() => setVisible(true));
    return () => window.cancelAnimationFrame(animationFrame);
  }, [entry.revision]);

  useEffect(() => {
    const left = remaining.current;
    if (left.revision !== entry.revision) {
      left.revision = entry.revision;
      left.ms = duration;
    }
    const ms = left.ms;
    if (held || ms === null) return;
    const startedAt = Date.now();
    const hideTimer = window.setTimeout(() => setVisible(false), ms);
    const removeTimer = window.setTimeout(
      () => dismiss(entry.id),
      ms + EXIT_DURATION_MS,
    );
    return () => {
      window.clearTimeout(hideTimer);
      window.clearTimeout(removeTimer);
      // A toast held while it fades out comes back, and one released stays
      // long enough to be read again.
      left.ms = Math.max(RESUME_MIN_MS, ms - (Date.now() - startedAt));
      setVisible(true);
    };
  }, [dismiss, duration, entry.id, entry.revision, held]);

  return (
    <div
      className={`toast toast-owned${visible ? ' show' : ''}`}
      data-toast-id={entry.id}
      data-toast-tone={toastTone(entry)}
      // Read whole when a deduplicated message replaces the text.
      aria-atomic="true"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget))
          setFocused(false);
      }}
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

/**
 * Shows the controller's messages above the page. Information toasts are
 * appended to a polite live region and warnings to an assertive one. Both
 * regions stay mounted while empty, because screen readers announce text
 * added to a region they already track and can skip a region inserted
 * together with its text. Each region has `aria-atomic="false"`, so a new
 * toast is read without the toasts already shown.
 */
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
  const hidden = useDocumentHidden();
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

  const toast = (entry: ToastEntry): ReactNode => (
    <ToastItem key={entry.id} entry={entry} dismiss={dismiss} hidden={hidden} />
  );
  const host = (
    <div id="toasts" className="toasts">
      <div
        className="toast-region"
        role="status"
        aria-live="polite"
        aria-atomic="false"
        aria-relevant="additions text"
      >
        {entries.filter((entry) => toastTone(entry) === 'info').map(toast)}
      </div>
      <div
        className="toast-region"
        role="alert"
        aria-live="assertive"
        aria-atomic="false"
        aria-relevant="additions text"
      >
        {entries.filter((entry) => toastTone(entry) === 'warning').map(toast)}
      </div>
    </div>
  );

  return (
    <ToastContext.Provider value={controller}>
      {children}
      {portalRoot ? createPortal(host, portalRoot) : host}
    </ToastContext.Provider>
  );
}
