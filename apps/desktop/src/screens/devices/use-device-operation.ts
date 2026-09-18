import { useEffect, useRef } from 'react';
import { AccessLifetime } from '../../app/access-lifetime';
import { useToast } from '/kit/toasts';
import { runDeviceMutation } from './operation-controller';
import { attemptRead } from '../../commands/command-policy';
import type { MutationPolicy } from '../../commands/command-policy';

export function useDeviceOperation(scope: string) {
  const toasts = useToast();
  const current = useRef({ scope, live: true, lifetime: new AccessLifetime() });
  if (current.current.scope !== scope) {
    current.current.live = false;
    current.current.lifetime.retire('access-change');
    current.current = { scope, live: true, lifetime: new AccessLifetime() };
  }
  useEffect(() => {
    const generation = current.current;
    generation.live = true;
    return () => {
      generation.live = false;
      generation.lifetime.retire('access-change');
    };
  }, [scope]);
  const capture = (): (() => boolean) => {
    const generation = current.current;
    const ticket = generation.lifetime.capture();
    return () =>
      ticket.isCurrent() && generation.live && current.current === generation;
  };
  const run = <T>(
    write: () => Promise<T>,
    onApplied: (value: T) => Promise<void>,
    onError: (error: unknown) => void,
    policy?: MutationPolicy,
  ) =>
    runDeviceMutation(
      write,
      onApplied,
      onError,
      () => {
        toasts.show('Change applied. Refresh pending.');
      },
      capture(),
      policy,
    );
  const settled = (callback: () => void): (() => void) => {
    const isCurrent = capture();
    return () => {
      if (isCurrent()) callback();
    };
  };
  const read = async <T>(
    task: () => Promise<T>,
    onRead: (value: T) => Promise<void>,
    onError: (error: unknown) => void,
  ) => {
    const isCurrent = capture();
    const result = await attemptRead({ kind: 'read-recovery' }, async () => {
      const value = await task();
      if (isCurrent()) await onRead(value);
      return value;
    });
    if (isCurrent() && result.outcome === 'read-failed') onError(result.error);
    return result;
  };
  return { run, read, capture, settled };
}
