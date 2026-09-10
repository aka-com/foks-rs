import { useCallback, useEffect, useRef, useState } from 'react';
import { enqueueProfileWork, normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatChannel,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../chat-contract';
import { failure, submissionId } from './actions';
import { reconcileOperations } from './operations';

/** Owns one mounted team's request lifetime and durable operation projection. */
export function useChatConversation(
  bridge: Bridge,
  profile: string,
  storeId: string,
) {
  const [channels, setChannels] = useState<ChatChannel[]>([]);
  const [pending, setPending] = useState<ChatOperation[]>([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [blocked, setBlocked] = useState('');
  const blockedRef = useRef('');
  const alive = useRef(false);
  const generation = useRef(0);
  const view = useRef(submissionId());
  const scope = useRef<ChatScope | null>(null);
  const request = useCallback(
    async (action: ChatAction): Promise<ChatReply> => {
      const epoch = generation.current;
      const viewId = view.current;
      return enqueueProfileWork(bridge, profile, async () => {
        if (!alive.current || generation.current !== epoch)
          throw new Error('Conversation closed.');
        if (blockedRef.current) throw new Error(blockedRef.current);
        let reply: ChatReply;
        try {
          reply = await bridge.chat(storeId, action, viewId);
        } catch (error) {
          const failure = normalizeCommandError(error);
          if (alive.current && generation.current === epoch && failure.fatal) {
            blockedRef.current = failure.message;
            setBlocked(failure.message);
            setChannels([]);
            setPending([]);
          }
          throw error;
        }
        if (!alive.current || generation.current !== epoch)
          throw new Error('Conversation closed.');
        if (
          scope.current &&
          JSON.stringify(scope.current) !== JSON.stringify(reply.scope)
        ) {
          const message =
            'The chat identity changed. Reopen this conversation.';
          blockedRef.current = message;
          setBlocked(message);
          setChannels([]);
          setPending([]);
          throw new Error(message);
        }
        scope.current = reply.scope;
        setPending((old) => reconcileOperations(old, action, reply.result));
        return reply;
      });
    },
    [bridge, profile, storeId],
  );
  const refreshPending = useCallback(async () => {
    await request({ action: 'pending' });
  }, [request]);
  const refresh = useCallback(async () => {
    const epoch = generation.current;
    setLoading(true);
    setError('');
    try {
      // Local recovery remains available when remote channel discovery fails.
      await refreshPending();
      const reply = await request({ action: 'channels' });
      if (reply.result.kind === 'channels') setChannels(reply.result.channels);
    } catch (e) {
      if (alive.current && generation.current === epoch) setError(failure(e));
    } finally {
      if (alive.current && generation.current === epoch) setLoading(false);
    }
  }, [request, refreshPending]);
  useEffect(() => {
    alive.current = true;
    view.current = submissionId();
    const viewId = view.current;
    const epoch = generation.current;
    void refresh();
    return () => {
      alive.current = false;
      generation.current = epoch + 1;
      scope.current = null;
      void bridge.cancelChat(viewId).catch(() => {});
    };
  }, [refresh, bridge]);
  return {
    channels,
    pending,
    error,
    loading,
    blocked,
    request,
    refresh,
    refreshPending,
  };
}
