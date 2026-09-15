import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatReply,
  ChatResult,
  ChatScope,
} from '../chat-contract';
import { CHAT_PENDING_ROWS } from '../chat-limits';
import {
  chatClient,
  cancelled,
  sameScope,
  integrity,
  channelIntegrity,
} from './client';
import { conversationResult, emptyConversation } from './conversation-model';
import { eventFromReply } from './conversation-events';
import { recoverPending } from './recover-pending';
import { coalesceStatus, RecoverySchedule } from './recovery-schedule';
import type { ConversationEvent } from './conversation-events';
import { useChatInbox } from './inbox-provider';
import type { Availability } from '../model';

/** Foreground operation owner. Account synchronization belongs to the shell. */
export function useChatConversation(
  bridge: Bridge,
  profile: string,
  storeId: string,
  access: () => Availability = () => ({ available: true }),
  accessGeneration = 0,
) {
  const { service, snapshot } = useChatInbox();
  const inbox = snapshot.get(storeId);
  const [model, setModel] = useState(emptyConversation);
  const current = useRef(model);
  const dispatch = useCallback((event: ConversationEvent) => {
    current.current = conversationResult(current.current, event);
    setModel(current.current);
  }, []);
  const [error, setError] = useState('');
  const [blocked, setBlocked] = useState('');
  const fatal = useRef('');
  const accessRef = useRef(access);
  const accessGenerationRef = useRef(accessGeneration);
  accessRef.current = access;
  accessGenerationRef.current = accessGeneration;
  const owner = useRef<ReturnType<typeof chatClient> | null>(null);
  const resolvedScope = useRef<ChatScope | null>(null);
  const historyClients = useRef(
    new Map<ReturnType<typeof chatClient>, string>(),
  );
  const cancelHistory = useCallback((channel?: string) => {
    for (const [client, id] of historyClients.current) {
      if (channel === undefined || id === channel) {
        client.dispose();
        historyClients.current.delete(client);
      }
    }
  }, []);
  const update = useCallback(
    (action: ChatAction, result: ChatResult) => {
      dispatch(eventFromReply(action, result));
    },
    [dispatch],
  );
  const performRequest = useCallback(
    async (action: ChatAction): Promise<ChatReply> => {
      const generation = accessGenerationRef.current;
      const assertAccess = (phase: 'before' | 'after'): void => {
        const availability = accessRef.current();
        if (
          generation !== accessGenerationRef.current ||
          !availability.available
        )
          throw {
            code: availability.available
              ? 'access-changed'
              : availability.reason,
            message:
              phase === 'after'
                ? 'Access changed after the chat operation was sent. Reconcile its saved operation before continuing.'
                : 'Access changed before the chat operation was sent.',
            fatal: false,
            ambiguous:
              phase === 'after' &&
              !['channels', 'history', 'pending', 'status'].includes(
                action.action,
              ),
            retryable: phase === 'before',
          };
      };
      assertAccess('before');
      const client = owner.current;
      const channel =
        'channel' in action
          ? action.channel
          : action.action === 'attempt'
            ? current.current.operations.find(
                (op) => op.id === action.operation,
              )?.channel
            : undefined;
      const checkChannel = () => {
        if (channel && service.isChannelBlocked(storeId, channel))
          throw channelIntegrity();
      };
      checkChannel();
      if (fatal.current) throw integrity(fatal.current);
      if (!client) throw cancelled();
      if (
        (action.action === 'prepare-message' ||
          action.action === 'prepare-channel') &&
        current.current.operations.length >= CHAT_PENDING_ROWS
      )
        throw {
          code: 'chat-limit',
          message: 'Finish saved operations before preparing more.',
          fatal: false,
          ambiguous: false,
          retryable: false,
        };
      let historyClient: ReturnType<typeof chatClient> | undefined;
      try {
        let transport = client;
        if (action.action === 'history') {
          historyClient = chatClient(bridge, profile, storeId);
          transport = historyClient;
          historyClients.current.set(historyClient, action.channel);
        }
        assertAccess('before');
        const reply = await transport.request(action, undefined, assertAccess);
        assertAccess('after');
        if (owner.current !== client) throw cancelled();
        const trusted = service.getSnapshot().get(storeId)?.scope;
        if (
          (trusted && !sameScope(trusted, reply.scope)) ||
          (resolvedScope.current &&
            !sameScope(resolvedScope.current, reply.scope))
        ) {
          service.block(storeId, 'The chat identity changed.');
          throw integrity();
        }
        resolvedScope.current = reply.scope;
        checkChannel();
        // History acknowledgment happens only after the history model accepts a page.
        if (reply.result.kind !== 'history') {
          update(action, reply.result);
          const channels = service.getSnapshot().get(storeId)?.data?.channels;
          if (channels)
            dispatch({
              kind: 'access',
              readable: new Set(
                channels
                  .filter(
                    (c) =>
                      c.readable && !service.isChannelBlocked(storeId, c.id),
                  )
                  .map((c) => c.id),
              ),
            });
        }
        return reply;
      } catch (cause) {
        if (owner.current === client) {
          const typed = normalizeCommandError(cause);
          if (typed.code === 'chat-channel-integrity' && channel) {
            service.blockChannel(storeId, channel);
            cancelHistory(channel);
          } else if (typed.fatal) {
            client.dispose();
            cancelHistory();
            owner.current = null;
            fatal.current = typed.message;
            setBlocked(typed.message);
            dispatch({ kind: 'reset' });
          }
          if (!typed.fatal && action.action === 'attempt')
            dispatch({
              kind: 'status-unresolved',
              operation: action.operation,
            });
          if (typed.code === 'chat-access-denied') service.invalidate(storeId);
        }
        throw cause;
      } finally {
        historyClient?.dispose();
        if (historyClient) historyClients.current.delete(historyClient);
      }
    },
    [bridge, profile, storeId, service, update, dispatch, cancelHistory],
  );
  const statusChecks = useRef(new Map<string, Promise<ChatReply>>());
  const schedule = useRef(new RecoverySchedule());
  // Automatic recovery admits one RPC at a time. Explicit requests enter the
  // profile queue before the next automatic item, bypassing scheduler backoff.
  const request = useCallback(
    (action: ChatAction): Promise<ChatReply> => {
      if (action.action !== 'status') return performRequest(action);
      return coalesceStatus(statusChecks.current, action.operation, () =>
        performRequest(action),
      );
    },
    [performRequest],
  );
  const recovery = useRef<Promise<void> | null>(null);
  const refreshPending = useCallback(() => {
    if (recovery.current) return recovery.current;
    const client = owner.current;
    const work = recoverPending(
      request,
      () => current.current.operations,
      () => client !== null && owner.current === client,
      schedule.current,
      (channel) => service.isChannelBlocked(storeId, channel),
    );
    recovery.current = work;
    void work
      .finally(() => {
        if (recovery.current === work) recovery.current = null;
      })
      .catch(() => {});
    return work;
  }, [request, service, storeId]);
  const refresh = useCallback(async () => {
    const client = owner.current;
    setError('');
    service.invalidate(storeId);
    try {
      await refreshPending();
    } catch (cause) {
      if (
        owner.current === client &&
        normalizeCommandError(cause).code !== 'cancelled'
      )
        setError(normalizeCommandError(cause).message);
    }
  }, [refreshPending, service, storeId]);
  const markRead = useCallback(
    async (channel: string, sequence: string) => {
      try {
        await request({ action: 'mark-read', channel, sequence });
      } finally {
        service.invalidate(storeId);
      }
    },
    [request, service, storeId],
  );
  const blockHistory = useCallback(
    (channel: string) => {
      service.blockChannel(storeId, channel);
      cancelHistory(channel);
    },
    [service, storeId, cancelHistory],
  );
  const acceptHistory = useCallback(
    (
      result: Extract<ChatResult, { kind: 'history' }>,
      before: string | null,
    ) => {
      update({ action: 'history', channel: result.channel, before }, result);
    },
    [update],
  );
  // Cached channels can mount a child history effect immediately. Establish
  // request ownership before passive effects in that child run.
  useLayoutEffect(() => {
    const client = chatClient(bridge, profile, storeId);
    owner.current = client;
    schedule.current = new RecoverySchedule();
    statusChecks.current = new Map();
    return () => {
      owner.current = null;
      client.dispose();
      cancelHistory();
      dispatch({ kind: 'reset' });
      recovery.current = null;
      resolvedScope.current = null;
    };
  }, [bridge, profile, storeId, cancelHistory, dispatch]);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  useEffect(() => {
    if (inbox?.data) {
      for (const channel of inbox.blockedChannels) cancelHistory(channel);
      const readable = new Set(
        inbox.data.channels
          .filter((c) => c.readable && !inbox.blockedChannels.has(c.id))
          .map((c) => c.id),
      );
      dispatch({ kind: 'access', readable });
      dispatch({ kind: 'channels', channels: inbox.data.channels });
    }
  }, [inbox?.data, inbox?.blockedChannels, dispatch, cancelHistory]);
  useEffect(() => {
    if (inbox?.state === 'blocked') {
      owner.current?.dispose();
      owner.current = null;
      cancelHistory();
      fatal.current = inbox.error;
      setBlocked(inbox.error);
      dispatch({ kind: 'reset' });
    }
    if (inbox?.state === 'unavailable' || inbox?.state === 'loading') {
      dispatch({ kind: 'access', readable: new Set() });
    }
  }, [inbox?.state, inbox?.error, dispatch, cancelHistory]);
  return {
    channels: inbox?.data?.channels ?? [],
    conversations: inbox?.data?.conversations ?? [],
    pending: model.operations,
    history: model.history,
    error:
      error ||
      (!inbox?.data || inbox?.state === 'unavailable'
        ? (inbox?.error ?? '')
        : ''),
    syncError: inbox?.data ? inbox.error : '',
    degraded: inbox?.data?.degraded ?? false,
    loading: !inbox || (inbox.state === 'loading' && !inbox.error),
    // A team whose next synchronization has not landed is still answering for
    // what it holds: a channel created a moment ago is listed by that reply.
    // A synchronization that failed says so instead, so this does not become a
    // wait with nothing behind it.
    resyncing:
      inbox?.state === 'ready' &&
      !inbox.error &&
      service.isInvalidated(storeId),
    blocked,
    blockedChannels: inbox?.blockedChannels ?? EMPTY_BLOCKED,
    revision: inbox?.revision ?? 0,
    channelRevisions: inbox?.channelRefreshRevisions ?? inbox?.channelRevisions,
    actor: inbox?.scope?.actor ?? null,
    request,
    refresh,
    refreshPending,
    markRead,
    acceptHistory,
    blockHistory,
  };
}

const EMPTY_BLOCKED: ReadonlySet<string> = new Set();
