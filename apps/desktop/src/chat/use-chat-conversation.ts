import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatReply,
  ChatResult,
  ChatScope,
} from '../chat-contract';
import {
  chatClient,
  cancelled,
  sameScope,
  integrity,
  channelIntegrity,
  type ChatWorkPriority,
} from './client';
import { useChatSends } from './send-provider';
import type { HistoryBinding } from './history-cache';
import { useChatInbox } from './inbox-provider';
import type { Availability } from '../model';

/** Foreground history owner. Submissions and account sync belong to the shell. */
export function useChatConversation(
  bridge: Bridge,
  profile: string,
  storeId: string,
  access: () => Availability = () => ({ available: true }),
  accessGeneration = 0,
) {
  const { service, snapshot } = useChatInbox();
  const { service: sends } = useChatSends();
  const inbox = snapshot.get(storeId);
  useSyncExternalStore(
    service.histories.subscribe,
    service.histories.getSnapshot,
    service.histories.getSnapshot,
  );
  const historyBindings = useRef(new WeakMap<ChatResult, HistoryBinding>());
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
  const performRequest = useCallback(
    async (action: ChatAction): Promise<ChatReply> => {
      if (action.action !== 'history') return sends.request(storeId, action);
      const generation = accessGenerationRef.current;
      const binding = service.histories.binding(
        storeId,
        action.channel,
        generation,
      );
      const assertAccess = (phase: 'before' | 'after'): void => {
        if (
          !binding ||
          service.histories.binding(storeId, action.channel, generation) !==
            binding
        )
          throw cancelled();
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
      const channel = action.channel;
      const checkChannel = () => {
        if (channel && service.isChannelBlocked(storeId, channel))
          throw channelIntegrity();
      };
      checkChannel();
      if (fatal.current) throw integrity(fatal.current);
      if (!client) throw cancelled();
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
        historyBindings.current.set(reply.result, binding!);
        return reply;
      } catch (cause) {
        if (owner.current === client) {
          const typed = normalizeCommandError(cause);
          service.handleError(storeId, typed, channel);
          if (typed.code === 'chat-channel-integrity' && channel) {
            service.blockChannel(storeId, channel);
            cancelHistory(channel);
          } else if (service.getSnapshot().get(storeId)?.state === 'blocked') {
            client.dispose();
            cancelHistory();
            owner.current = null;
            fatal.current = typed.message;
            setBlocked(typed.message);
            service.histories.clear(storeId);
          }
          if (typed.code === 'chat-access-denied') service.invalidate(storeId);
        }
        throw cause;
      } finally {
        historyClient?.dispose();
        if (historyClient) historyClients.current.delete(historyClient);
      }
    },
    [bridge, profile, storeId, service, sends, cancelHistory],
  );
  // Automatic recovery admits one RPC at a time. Explicit requests enter the
  // profile queue before the next automatic item, bypassing scheduler backoff.
  const request = performRequest;
  const refreshPending = useCallback(
    () => sends.refresh(storeId),
    [sends, storeId],
  );
  const refresh = useCallback(
    async (priority: ChatWorkPriority = 'foreground') => {
      const client = owner.current;
      setError('');
      service.invalidate(storeId);
      try {
        await sends.refresh(storeId, priority);
      } catch (cause) {
        if (
          owner.current === client &&
          normalizeCommandError(cause).code !== 'cancelled'
        )
          setError(normalizeCommandError(cause).message);
      }
    },
    [sends, service, storeId],
  );
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
      const binding = historyBindings.current.get(result);
      if (
        !binding ||
        binding.generation !== accessGenerationRef.current ||
        !accessRef.current().available
      )
        throw cancelled();
      service.histories.accept(binding, result, before);
      sends.observeHistory(storeId, result.channel, result.messages);
    },
    [service, sends, storeId],
  );
  // Cached channels can mount a child history effect immediately. Establish
  // request ownership before passive effects in that child run.
  useLayoutEffect(() => {
    const client = chatClient(bridge, profile, storeId);
    owner.current = client;
    return () => {
      owner.current = null;
      client.dispose();
      cancelHistory();
      historyBindings.current = new WeakMap();
      resolvedScope.current = null;
    };
  }, [bridge, profile, storeId, cancelHistory]);
  useEffect(() => {
    void refresh('background');
  }, [refresh]);
  useEffect(() => {
    if (inbox?.data) {
      for (const channel of inbox.blockedChannels) cancelHistory(channel);
    }
  }, [inbox?.data, inbox?.blockedChannels, cancelHistory]);
  useEffect(() => {
    if (inbox?.state === 'blocked') {
      owner.current?.dispose();
      owner.current = null;
      cancelHistory();
      fatal.current = inbox.error;
      setBlocked(inbox.error);
    }
    if (inbox?.state === 'unavailable' || inbox?.state === 'loading')
      cancelHistory();
  }, [inbox?.state, inbox?.error, cancelHistory]);
  return {
    channels: inbox?.data?.channels ?? [],
    channelsKnown: inbox?.state === 'ready' && Boolean(inbox.data),
    conversations: inbox?.data?.conversations ?? [],
    pending: sends.operations(storeId),
    history: (channel: string) =>
      access().available
        ? service.histories.get(
            service.histories.binding(storeId, channel, accessGeneration),
          )
        : null,
    error:
      error ||
      (!inbox?.data || inbox?.state === 'unavailable'
        ? (inbox?.error ?? '')
        : ''),
    syncError: inbox?.data ? inbox.error : '',
    note: inbox?.data ? inbox.note : '',
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
    scope: inbox?.scope ?? null,
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
