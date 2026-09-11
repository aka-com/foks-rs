import { useCallback, useEffect, useRef, useState } from 'react';
import { enqueueProfileWork, normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatChannel,
  ChatConversation,
  ChatOperation,
  ChatReply,
  ChatScope,
} from '../chat-contract';
import { failure, submissionId } from './actions';
import { reconcileOperations } from './operations';

const wait = (milliseconds: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, milliseconds));
const backoff = (milliseconds: number) =>
  Math.min(
    5_000,
    milliseconds + Math.floor(Math.random() * Math.max(1, milliseconds / 4)),
  );

/** Owns one mounted team's request lifetime and durable operation projection. */
export function useChatConversation(
  bridge: Bridge,
  profile: string,
  storeId: string,
) {
  const [channels, setChannels] = useState<ChatChannel[]>([]);
  const [conversations, setConversations] = useState<ChatConversation[]>([]);
  const [pending, setPending] = useState<ChatOperation[]>([]);
  const [error, setError] = useState('');
  const [syncError, setSyncError] = useState('');
  const [degraded, setDegraded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [blocked, setBlocked] = useState('');
  const [revision, setRevision] = useState(0);
  const [actor, setActor] = useState<string | null>(null);
  const blockedRef = useRef('');
  const alive = useRef(false);
  const generation = useRef(0);
  const view = useRef(submissionId());
  const scope = useRef<ChatScope | null>(null);
  const head = useRef('0');
  const syncing = useRef<Promise<ChatReply> | null>(null);
  const accept = useCallback(
    (reply: ChatReply, action: ChatAction, epoch: number) => {
      if (!alive.current || generation.current !== epoch)
        throw new Error('Conversation closed.');
      if (
        scope.current &&
        JSON.stringify(scope.current) !== JSON.stringify(reply.scope)
      ) {
        const message = 'The chat identity changed. Reopen this conversation.';
        blockedRef.current = message;
        setBlocked(message);
        setChannels([]);
        setConversations([]);
        setPending([]);
        throw new Error(message);
      }
      if (!scope.current) setActor(reply.scope.actor);
      scope.current = reply.scope;
      setPending((old) => reconcileOperations(old, action, reply.result));
      if (reply.result.kind === 'inbox') {
        head.current = reply.result.head;
        setConversations(reply.result.conversations);
        setDegraded(reply.result.degraded);
        setRevision((value) => value + 1);
        setSyncError('');
      }
      return reply;
    },
    [],
  );
  const handleFailure = useCallback((cause: unknown, epoch: number) => {
    const normalized = normalizeCommandError(cause);
    if (alive.current && generation.current === epoch && normalized.fatal) {
      blockedRef.current = normalized.message;
      setBlocked(normalized.message);
      setChannels([]);
      setConversations([]);
      setPending([]);
    }
    return normalized;
  }, []);
  const request = useCallback(
    async (action: ChatAction): Promise<ChatReply> => {
      const epoch = generation.current;
      const viewId = view.current;
      return enqueueProfileWork(bridge, profile, async () => {
        if (!alive.current || generation.current !== epoch)
          throw new Error('Conversation closed.');
        if (blockedRef.current) throw new Error(blockedRef.current);
        try {
          return accept(
            await bridge.chat(storeId, action, viewId),
            action,
            epoch,
          );
        } catch (cause) {
          handleFailure(cause, epoch);
          throw cause;
        }
      });
    },
    [accept, bridge, handleFailure, profile, storeId],
  );
  const syncInbox = useCallback((): Promise<ChatReply> => {
    if (syncing.current) return syncing.current;
    const run = request({ action: 'sync-inbox' });
    syncing.current = run;
    void run.then(
      () => {
        if (syncing.current === run) syncing.current = null;
      },
      () => {
        if (syncing.current === run) syncing.current = null;
      },
    );
    return run;
  }, [request]);
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
      await syncInbox();
    } catch (cause) {
      if (alive.current && generation.current === epoch)
        setError(failure(cause));
    } finally {
      if (alive.current && generation.current === epoch) setLoading(false);
    }
  }, [request, refreshPending, syncInbox]);
  const markRead = useCallback(
    async (channel: string, sequence: string) => {
      await request({ action: 'mark-read', channel, sequence });
      await syncInbox();
    },
    [request, syncInbox],
  );
  const poll = useCallback(
    async (epoch: number, viewId: string) => {
      let retry = 250;
      while (alive.current && generation.current === epoch) {
        const action: ChatAction = {
          action: 'poll-inbox',
          since: head.current,
          timeout_milliseconds: 25_000,
        };
        try {
          const reply = accept(
            await bridge.chat(storeId, action, viewId),
            action,
            epoch,
          );
          if (reply.result.kind === 'poll' && reply.result.bumped) {
            await syncInbox();
            if (BigInt(head.current) <= BigInt(action.since)) {
              setSyncError(
                'Live updates did not advance. Retrying more slowly.',
              );
              await wait(backoff(retry));
              retry = Math.min(retry * 2, 5_000);
              continue;
            }
          }
          retry = 250;
          setSyncError('');
        } catch (cause) {
          if (!alive.current || generation.current !== epoch) return;
          const normalized = handleFailure(cause, epoch);
          if (normalized.fatal) return;
          setSyncError(normalized.message);
          await wait(backoff(retry));
          retry = Math.min(retry * 2, 5_000);
        }
      }
    },
    [accept, bridge, handleFailure, storeId, syncInbox],
  );
  useEffect(() => {
    alive.current = true;
    view.current = submissionId();
    const viewId = view.current;
    const epoch = generation.current;
    void refresh().then(() => poll(epoch, viewId));
    return () => {
      alive.current = false;
      generation.current = epoch + 1;
      scope.current = null;
      syncing.current = null;
      void bridge.cancelChat(viewId).catch(() => {});
    };
  }, [refresh, poll, bridge]);
  return {
    channels,
    conversations,
    pending,
    error,
    syncError,
    degraded,
    loading,
    blocked,
    revision,
    actor,
    request,
    refresh,
    refreshPending,
    syncInbox,
    markRead,
  };
}
