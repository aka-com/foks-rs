import { useEffect, useState } from 'react';
import type { Bridge } from '../bridge';

const recoveries = new WeakMap<
  Bridge,
  { generation: number; attempt: number; work: ReturnType<Bridge['chatLocal']> }
>();
function recover(bridge: Bridge, generation: number, attempt: number) {
  const previous = recoveries.get(bridge);
  if (previous?.generation === generation && previous.attempt === attempt)
    return previous.work;
  const work = bridge.chatLocal({ action: 'recover-intents' });
  recoveries.set(bridge, { generation, attempt, work });
  return work;
}

/** Recovery belongs to the unlocked shell, never a channel visit. Native IPC
 * also gates sends, so neither an early render nor a stale view can bypass it. */
export function useChatMigration(
  bridge: Bridge,
  enabled: boolean,
  generation: number,
) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    bridge: Bridge | null;
    generation: number;
    attempt: number;
    ready: boolean;
    failed: boolean;
  }>({
    bridge: null,
    generation: -1,
    attempt: -1,
    ready: false,
    failed: false,
  });
  useEffect(() => {
    if (!enabled || !bridge.native) return;
    let current = true;
    void recover(bridge, generation, attempt).then(
      (session) => {
        if (current)
          setState({
            bridge,
            generation,
            attempt,
            ready: session.migrationRemaining === 0,
            failed: session.migrationRemaining !== 0,
          });
      },
      () => {
        if (current)
          setState({ bridge, generation, attempt, ready: false, failed: true });
      },
    );
    return () => {
      current = false;
    };
  }, [bridge, enabled, generation, attempt]);
  const current =
    state.bridge === bridge &&
    state.generation === generation &&
    state.attempt === attempt;
  return {
    ready: !bridge.native || (enabled && current && state.ready),
    notice:
      !bridge.native || !enabled || (current && state.ready) ? null : (
        <div className="chat-migration" role="status">
          {current && state.failed ? (
            <>
              <span>
                Saved messages need recovery before chat can resume. Your
                messages have been kept.
              </span>
              <button
                type="button"
                onClick={() => setAttempt((value) => value + 1)}
              >
                Retry recovery
              </button>
            </>
          ) : (
            <span>Recovering saved messages…</span>
          )}
        </div>
      ),
  };
}
