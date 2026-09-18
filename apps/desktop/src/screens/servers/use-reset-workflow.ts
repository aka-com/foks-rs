import { useEffect, useRef, useSyncExternalStore } from 'react';
import type { Bridge } from '../../bridge';
import type { Server } from '../../model';
import { serverBinding } from './server-workflow';
import { ResetWorkflow } from './reset-workflow';

export function useResetWorkflow({
  bridge,
  server,
  onClose,
  onPreviewError,
  onError,
}: {
  bridge: Bridge;
  server: Server;
  onClose: () => void;
  onPreviewError: (error: unknown) => void;
  onError: (error: unknown) => void;
}) {
  const binding = serverBinding(server);
  const latest = useRef({ bridge, binding, onClose, onPreviewError, onError });
  latest.current = { bridge, binding, onClose, onPreviewError, onError };
  const owner = useRef<{
    bridge: Bridge;
    binding: string;
    workflow: ResetWorkflow;
  } | null>(null);
  if (
    !owner.current ||
    owner.current.bridge !== bridge ||
    owner.current.binding !== binding
  ) {
    owner.current?.workflow.retire();
    owner.current = {
      bridge,
      binding,
      workflow: new ResetWorkflow(
        bridge,
        server.id,
        () =>
          latest.current.bridge === bridge && latest.current.binding === binding,
        (error) => latest.current.onPreviewError(error),
        (error) => latest.current.onError(error),
      ),
    };
  }
  const workflow = owner.current.workflow;
  const state = useSyncExternalStore(
    workflow.subscribe,
    workflow.getSnapshot,
    workflow.getSnapshot,
  );
  // Reset preview tokens are invalidated when the window loses focus.
  useEffect(() => {
    workflow.activate();
    void workflow.load();
    const conceal = () => {
      workflow.retire();
      latest.current.onClose();
    };
    const concealWhenHidden = () => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      workflow.retire();
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, [workflow]);
  return {
    ...state,
    load: workflow.load,
    reset: (confirmation: string, onReset: () => Promise<void>) =>
      workflow.reset(confirmation, onReset),
    close: () => {
      if (workflow.getSnapshot().busy) return;
      workflow.retire();
      latest.current.onClose();
    },
  };
}
