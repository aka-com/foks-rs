import { useEffect, useState } from 'react';
import type { Bridge } from './bridge';

export const SETUP_SLOW_AFTER_MS = 8_000;

/** Delay before displaying the slow-operation state; it does not cancel or retry the operation. */
export function useSlowSetup(active: boolean): boolean {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (!active) {
      setSlow(false);
      return;
    }
    const timer = window.setTimeout(() => setSlow(true), SETUP_SLOW_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [active]);
  return active && slow;
}

const reads = new WeakMap<Bridge, Map<string, Promise<unknown>>>();

/** Reuses active reads across component remounts to prevent duplicate requests. */
export function sharedSetupRead<T>(
  bridge: Bridge,
  key: string,
  read: () => Promise<T>,
): Promise<T> {
  let active = reads.get(bridge);
  if (!active) {
    active = new Map();
    reads.set(bridge, active);
  }
  const existing = active.get(key);
  if (existing) return existing as Promise<T>;
  const promise = Promise.resolve()
    .then(read)
    .finally(() => {
      if (active.get(key) === promise) active.delete(key);
    });
  active.set(key, promise);
  return promise;
}
