import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { normalizeCommandError } from '../bridge';
import type { Bridge, CommandError } from '../bridge';
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
import { accessChanged } from './errors';
import { chatErrorText } from './error-copy';
import { useChatSends } from './send-provider';
import { markChannelRead } from './mark-read';
import type { HistoryBinding, ListPosition } from './history-cache';
import type { HistoryPlacement } from './conversation-model';
import { useChatInbox } from './inbox-provider';
import { settleRechecks, type Recheck } from './recheck';
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
  const [failure, setFailure] = useState<CommandError | null>(null);
  const [blocked, setBlocked] = useState('');
  // The failure that stopped the session, for its diagnostic details.
  const [blockedFailure, setBlockedFailure] = useState<CommandError | null>(
    null,
  );
  const fatal = useRef('');
  // Read by history reads and acceptance, which run after the commit that
  // rendered them, never during a render.
  const accessRef = useRef(access);
  const accessGenerationRef = useRef(accessGeneration);
  useLayoutEffect(() => {
    accessRef.current = access;
    accessGenerationRef.current = accessGeneration;
  });
  // The mounted request owner. It is replaced when the bridge, profile or
  // store changes and cleared when the team is blocked; a history reply that
  // returns to an earlier owner is discarded.
  const owner = useRef<object | null>(null);
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
  const readHistory = useCallback(
    async (
      action: Extract<ChatAction, { action: 'history' }>,
    ): Promise<ChatReply> => {
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
          throw accessChanged(
            phase === 'after'
              ? 'Access changed after the chat history read was sent.'
              : 'Access changed before the chat history read was sent.',
            phase,
            'read',
          );
      };
      assertAccess('before');
      const token = owner.current;
      const channel = action.channel;
      const checkChannel = () => {
        if (channel && service.isChannelBlocked(storeId, channel))
          throw channelIntegrity();
      };
      checkChannel();
      if (fatal.current) throw integrity(fatal.current);
      if (!token) throw cancelled();
      // Each read has its own client, so blocking one channel cancels only
      // that channel's reads.
      const client = chatClient(bridge, profile, storeId);
      historyClients.current.set(client, channel);
      try {
        assertAccess('before');
        const reply = await client.request(action, undefined, assertAccess);
        assertAccess('after');
        if (owner.current !== token) throw cancelled();
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
        if (owner.current === token) {
          const typed = normalizeCommandError(cause);
          service.handleError(storeId, typed, channel);
          if (typed.code === 'chat-channel-integrity' && channel) {
            service.blockChannel(storeId, channel, typed);
            cancelHistory(channel);
          } else if (service.getSnapshot().get(storeId)?.state === 'blocked') {
            cancelHistory();
            owner.current = null;
            fatal.current = chatErrorText(typed);
            setBlocked(fatal.current);
            setBlockedFailure(typed);
            service.histories.clear(storeId);
          }
          if (typed.code === 'chat-access-denied') service.invalidate(storeId);
        }
        throw cause;
      } finally {
        client.dispose();
        historyClients.current.delete(client);
      }
    },
    [bridge, profile, storeId, service, cancelHistory],
  );
  // History reads belong to this conversation. Every other action goes to the
  // send service, which owns operation state and the profile queue.
  const request = useCallback(
    (action: ChatAction): Promise<ChatReply> =>
      action.action === 'history'
        ? readHistory(action)
        : sends.request(storeId, action),
    [readHistory, sends, storeId],
  );
  const refreshPending = useCallback(
    () => sends.refresh(storeId),
    [sends, storeId],
  );
  const refresh = useCallback(
    async (priority: ChatWorkPriority = 'foreground') => {
      const token = owner.current;
      setFailure(null);
      service.invalidate(storeId);
      try {
        await sends.refresh(storeId, priority);
      } catch (cause) {
        if (
          owner.current === token &&
          normalizeCommandError(cause).code !== 'cancelled'
        )
          setFailure(normalizeCommandError(cause));
      }
    },
    [sends, service, storeId],
  );
  const markRead = useCallback(
    (channel: string, sequence: string) =>
      markChannelRead(
        (action) => sends.request(storeId, action),
        service,
        storeId,
        channel,
        sequence,
      ),
    [sends, service, storeId],
  );
  // Opening a channel is a use of its retained history. `history` is read
  // while rendering, so it leaves the eviction order alone; the open records
  // the use instead.
  const setOpenChannel = useCallback(
    (channel: string | null) => {
      service.setOpenChannel(storeId, channel);
      if (channel !== null)
        service.histories.touch(
          service.histories.binding(
            storeId,
            channel,
            accessGenerationRef.current,
          ),
        );
    },
    [service, storeId],
  );
  // Read when a thread opens, during its first render, so it takes the
  // generation that render was given rather than the ref. A thread resumes
  // only in a window that `history` returned, and `history` returns none
  // while access is unavailable.
  const listPosition = useCallback(
    (channel: string) =>
      service.histories.position(
        service.histories.binding(storeId, channel, accessGeneration),
      ),
    [accessGeneration, service, storeId],
  );
  const keepListPosition = useCallback(
    (channel: string, position: ListPosition | null) =>
      service.histories.setPosition(
        service.histories.binding(
          storeId,
          channel,
          accessGenerationRef.current,
        ),
        position,
      ),
    [service, storeId],
  );
  // The channels the reader rechecked, until each recheck settles; see
  // `settleRechecks`.
  const [rechecks, setRechecks] = useState<ReadonlyMap<string, Recheck>>(
    () => new Map(),
  );
  // The reader's Recheck of a stopped channel: the inbox withdraws its block
  // and synchronizes again, and the thread that mounts reads the channel's
  // history from the start.
  const recheckChannel = useCallback(
    (channel: string) => {
      const revision =
        service.getSnapshot().get(storeId)?.authorizationRevision ?? 0;
      service.unblockChannel(storeId, channel);
      setRechecks((old) =>
        new Map(old).set(channel, { stopped: null, revision, read: false }),
      );
    },
    [service, storeId],
  );
  const blockHistory = useCallback(
    (channel: string, cause?: CommandError) => {
      service.blockChannel(storeId, channel, cause);
      cancelHistory(channel);
    },
    [service, storeId, cancelHistory],
  );
  const blockedChannels = inbox?.blockedChannels ?? EMPTY_BLOCKED;
  const authorizationRevision = inbox?.authorizationRevision ?? 0;
  useEffect(() => {
    setRechecks((old) =>
      settleRechecks(old, blockedChannels, authorizationRevision),
    );
  }, [blockedChannels, authorizationRevision]);
  const channelBlockCause = useCallback(
    (channel: string) => service.channelBlockCause(storeId, channel),
    [service, storeId],
  );
  const acceptHistory = useCallback(
    (
      result: Extract<ChatResult, { kind: 'history' }>,
      before: string | null,
      placement?: HistoryPlacement,
      sources?: readonly ChatResult[],
    ) => {
      // A page merged from narrowed reads is a new object: it is accepted
      // under its replies' binding when they all share one, and when its
      // rows, cursor and missing predecessors all came from them.
      let binding = historyBindings.current.get(result);
      if (sources?.length) {
        const pages = sources.filter((s) => s.kind === 'history');
        const bound = sources.map((s) => historyBindings.current.get(s));
        const rows = new Set(pages.flatMap((s) => s.messages));
        const missing = new Set(pages.flatMap((s) => s.missing_predecessors));
        binding =
          pages.length === sources.length &&
          bound.every((b) => b === bound[0]) &&
          pages.some((s) => s.before === result.before) &&
          result.messages.every((m) => rows.has(m)) &&
          result.missing_predecessors.every((m) => missing.has(m))
            ? bound[0]
            : undefined;
      }
      if (
        !binding ||
        binding.generation !== accessGenerationRef.current ||
        !accessRef.current().available
      )
        throw cancelled();
      const accepted = service.histories.accept(
        binding,
        result,
        before,
        placement,
      );
      sends.observeHistory(storeId, result.channel, result.messages);
      const entry = service.getSnapshot().get(storeId);
      setRechecks((old) =>
        settleRechecks(
          old,
          entry?.blockedChannels ?? EMPTY_BLOCKED,
          entry?.authorizationRevision ?? 0,
          result.channel,
        ),
      );
      return accepted;
    },
    [service, sends, storeId],
  );
  // Cached channels can mount a child history effect immediately. Establish
  // request ownership before passive effects in that child run. A new bridge,
  // profile or store is a new owner.
  useLayoutEffect(() => {
    owner.current = {};
    return () => {
      owner.current = null;
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
      owner.current = null;
      cancelHistory();
      fatal.current = inbox.error;
      setBlocked(inbox.error);
      setBlockedFailure(inbox.failure ?? null);
    }
    if (inbox?.state === 'unavailable' || inbox?.state === 'loading')
      cancelHistory();
  }, [inbox?.state, inbox?.error, inbox?.failure, cancelHistory]);
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
      (failure ? chatErrorText(failure) : '') ||
      (!inbox?.data || inbox?.state === 'unavailable'
        ? (inbox?.error ?? '')
        : ''),
    // The same failures with their codes, for callers that present a cause
    // differently from a plain fault.
    failure:
      failure ??
      (!inbox?.data || inbox?.state === 'unavailable'
        ? (inbox?.failure ?? null)
        : null),
    syncError: inbox?.data ? inbox.error : '',
    syncFailure: inbox?.data ? (inbox.failure ?? null) : null,
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
    blockedFailure,
    blockedChannels,
    /** The channels the reader rechecked; see `settleRechecks`. */
    rechecked: rechecks,
    channelBlockCause,
    revision: inbox?.revision ?? 0,
    channelRevisions: inbox?.channelRefreshRevisions ?? inbox?.channelRevisions,
    scope: inbox?.scope ?? null,
    actor: inbox?.scope?.actor ?? null,
    request,
    refresh,
    refreshPending,
    markRead,
    setOpenChannel,
    acceptHistory,
    blockHistory,
    recheckChannel,
    listPosition,
    keepListPosition,
  };
}

const EMPTY_BLOCKED: ReadonlySet<string> = new Set();
