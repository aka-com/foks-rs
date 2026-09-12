import { useCallback, useEffect, useRef, useState } from 'react';
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
import type { TrackedOperation } from './operations';
import { useChatInbox } from './inbox-provider';

/** Foreground operation owner. Account synchronization belongs to the shell. */
export function useChatConversation(
  bridge: Bridge,
  profile: string,
  storeId: string,
) {
  const { service, snapshot } = useChatInbox();
  const inbox = snapshot.get(storeId);
  const [model, setModel] = useState(emptyConversation);
  const current = useRef(model);
  const setOperations = useCallback((operations: TrackedOperation[]) => {
    current.current = { ...current.current, operations };
    setModel(current.current);
  }, []);
  const [error, setError] = useState('');
  const [blocked, setBlocked] = useState('');
  const fatal = useRef('');
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
  const update = useCallback((action: ChatAction, result: ChatResult) => {
    current.current = conversationResult(current.current, action, result);
    setModel(current.current);
  }, []);
  const request = useCallback(
    async (action: ChatAction): Promise<ChatReply> => {
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
        const reply = await transport.request(action);
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
            setOperations(
              current.current.operations.map((op) =>
                channels.some((c) => c.id === op.channel && c.readable) &&
                !service.isChannelBlocked(storeId, op.channel)
                  ? op
                  : { ...op, text: undefined },
              ),
            );
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
            current.current = emptyConversation();
            setOperations([]);
          }
          if (!typed.fatal && action.action === 'attempt')
            setOperations(
              current.current.operations.map((op) =>
                op.id === action.operation
                  ? { ...op, statusUnknown: true }
                  : op,
              ),
            );
          if (typed.code === 'chat-access-denied') service.invalidate(storeId);
        }
        throw cause;
      } finally {
        historyClient?.dispose();
        if (historyClient) historyClients.current.delete(historyClient);
      }
    },
    [bridge, profile, storeId, service, update, setOperations, cancelHistory],
  );
  const recovery = useRef<Promise<void> | null>(null);
  const refreshPending = useCallback(() => {
    if (recovery.current) return recovery.current;
    const work = (async () => {
      await request({ action: 'pending' });
      // Bound automatic status recovery; remaining rows retain explicit controls.
      for (const op of current.current.operations
        .filter((op) => op.statusUnknown && !op.observed)
        .slice(0, 16)) {
        try {
          await request({ action: 'status', operation: op.id });
        } catch (cause) {
          if (
            normalizeCommandError(cause).fatal ||
            normalizeCommandError(cause).code === 'cancelled'
          )
            break;
        }
      }
    })();
    recovery.current = work;
    void work
      .finally(() => {
        if (recovery.current === work) recovery.current = null;
      })
      .catch(() => {});
    return work;
  }, [request]);
  const syncInbox = useCallback(async () => {
    service.invalidate(storeId);
  }, [service, storeId]);
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
  useEffect(() => {
    const client = chatClient(bridge, profile, storeId);
    owner.current = client;
    void refresh();
    return () => {
      owner.current = null;
      client.dispose();
      cancelHistory();
      current.current = emptyConversation();
      recovery.current = null;
      resolvedScope.current = null;
    };
  }, [bridge, profile, storeId, refresh, cancelHistory]);
  useEffect(() => {
    if (inbox?.data) {
      for (const channel of inbox.blockedChannels) cancelHistory(channel);
      const readable = new Set(
        inbox.data.channels
          .filter((c) => c.readable && !inbox.blockedChannels.has(c.id))
          .map((c) => c.id),
      );
      current.current = {
        ...current.current,
        operations: current.current.operations.map((op) =>
          readable.has(op.channel) ? op : { ...op, text: undefined },
        ),
        history:
          current.current.history &&
          readable.has(current.current.history.channel)
            ? current.current.history
            : null,
      };
      update(
        { action: 'channels' },
        { kind: 'channels', channels: inbox.data.channels, version: '0' },
      );
    }
  }, [inbox?.data, inbox?.blockedChannels, update, cancelHistory]);
  useEffect(() => {
    if (inbox?.state === 'blocked') {
      owner.current?.dispose();
      owner.current = null;
      cancelHistory();
      fatal.current = inbox.error;
      setBlocked(inbox.error);
      current.current = emptyConversation();
      setOperations([]);
    }
    if (inbox?.state === 'unavailable' || inbox?.state === 'loading') {
      current.current = { ...current.current, history: null };
      current.current.operations = current.current.operations.map((op) => ({
        ...op,
        text: undefined,
      }));
      setOperations(current.current.operations);
    }
  }, [inbox?.state, inbox?.error, setOperations, cancelHistory]);
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
    blocked,
    blockedChannels: inbox?.blockedChannels ?? EMPTY_BLOCKED,
    revision: inbox?.revision ?? 0,
    actor: inbox?.scope?.actor ?? null,
    request,
    refresh,
    refreshPending,
    syncInbox,
    markRead,
    acceptHistory,
    blockHistory,
  };
}

const EMPTY_BLOCKED: ReadonlySet<string> = new Set();
