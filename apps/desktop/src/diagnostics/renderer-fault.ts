/**
 * Records renderer faults in the timing log: render errors an error boundary
 * caught, errors React could not hand to a boundary, and script errors and
 * unhandled promise rejections the window reported.
 *
 * Each fault is one `renderer.fault` event whose `boundary` attribute is the
 * static label of the boundary or listener that observed it. The event never
 * carries the error message or a component stack: production builds mangle
 * component and custom class names, and the log forbids messages. `code` is
 * the constructor name of a built-in error type, the one name minification
 * leaves intact, and is absent for anything else.
 */

import type { RootOptions } from 'react-dom/client';
import { diagnosticLog, type DiagnosticLog, type TimingEvent } from './log';

export const RENDERER_FAULT = 'renderer.fault';

/**
 * Faults recorded per label in one window load. A fault repeated on every
 * frame would otherwise displace every other entry from the bounded log.
 */
export const FAULTS_PER_LABEL = 32;

/** A boundary that records its own faults carries its label here. */
export interface LabelledBoundary {
  readonly faultLabel: string;
}

function builtInErrors(): readonly (abstract new (
  ...args: never[]
) => object)[] {
  const scope = globalThis as {
    AggregateError?: abstract new (...args: never[]) => object;
    DOMException?: abstract new (...args: never[]) => object;
  };
  return [
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    URIError,
    ...(scope.AggregateError ? [scope.AggregateError] : []),
    ...(scope.DOMException ? [scope.DOMException] : []),
  ];
}

/**
 * The constructor name of a built-in error, or undefined for a subclass, a
 * non-error value or an error from another realm.
 */
export function faultCode(error: unknown): string | undefined {
  if (typeof error !== 'object' || error === null) return undefined;
  const constructor = (Object.getPrototypeOf(error) as object | null)
    ?.constructor;
  return builtInErrors().find((type) => type === constructor)?.name;
}

const recorded = new WeakMap<DiagnosticLog, Map<string, number>>();

/**
 * Records one fault under `label`, or nothing once that label has reached
 * `FAULTS_PER_LABEL` in this log.
 */
export function recordRendererFault(
  label: string,
  error: unknown,
  log: DiagnosticLog = diagnosticLog,
): TimingEvent | undefined {
  const counts = recorded.get(log) ?? new Map<string, number>();
  recorded.set(log, counts);
  const count = counts.get(label) ?? 0;
  if (count >= FAULTS_PER_LABEL) return undefined;
  counts.set(label, count + 1);
  const code = faultCode(error);
  return log.record({
    name: RENDERER_FAULT,
    outcome: 'error',
    ...(code ? { code } : {}),
    attrs: { boundary: label },
  });
}

function recordsOwnFaults(boundary: unknown): boolean {
  return (
    typeof boundary === 'object' &&
    boundary !== null &&
    typeof (boundary as Partial<LabelledBoundary>).faultLabel === 'string'
  );
}

/**
 * The `createRoot` error callbacks. A labelled boundary has already recorded
 * and logged its fault; any other boundary's fault, an error no boundary
 * caught and an error React recovered from are recorded here and written to
 * the console as React's defaults would.
 */
export function rendererRootOptions(
  log: DiagnosticLog = diagnosticLog,
  report: (...args: unknown[]) => void = console.error,
): Pick<
  RootOptions,
  'onUncaughtError' | 'onCaughtError' | 'onRecoverableError'
> {
  return {
    onUncaughtError: (error, info) => {
      recordRendererFault('uncaught', error, log);
      report('The window failed to render.', error, info.componentStack);
    },
    onCaughtError: (error, info) => {
      if (recordsOwnFaults(info.errorBoundary)) return;
      recordRendererFault('unlabelled', error, log);
      report('A component failed to render.', error, info.componentStack);
    },
    onRecoverableError: (error, info) => {
      recordRendererFault('recoverable', error, log);
      report('React recovered from an error.', error, info.componentStack);
    },
  };
}

/**
 * Records the window's script errors and unhandled promise rejections. The
 * listeners leave the events' default handling in place, so the console
 * still reports them. Answers the function that removes both listeners.
 */
export function listenForWindowFaults(
  target: Pick<Window, 'addEventListener' | 'removeEventListener'>,
  log: DiagnosticLog = diagnosticLog,
): () => void {
  // A script error carries the thrown value. An event without one is a
  // notice such as the ResizeObserver loop limit, or a cross-origin script
  // error reported without detail, and is not recorded.
  const error = (event: ErrorEvent): void => {
    if (event.error === null || event.error === undefined) return;
    recordRendererFault('window-error', event.error, log);
  };
  const rejection = (event: PromiseRejectionEvent): void => {
    recordRendererFault('unhandled-rejection', event.reason, log);
  };
  target.addEventListener('error', error);
  target.addEventListener('unhandledrejection', rejection);
  return () => {
    target.removeEventListener('error', error);
    target.removeEventListener('unhandledrejection', rejection);
  };
}
