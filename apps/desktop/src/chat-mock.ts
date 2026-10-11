import type { AgentSnapshot } from './model';
import { cancelled } from './chat/client';
import { CHAT_PAGE_ROWS, CHAT_SNIPPET_BYTES } from './chat-limits';
import {
  decodeChatOperation,
  decodeChatReplyFor,
  type ChatAction,
  type ChatChannel,
  type ChatMessage,
  type ChatOperation,
  type ChatReply,
  type ChatResult,
  type ChatTeamRef,
} from './chat-contract';
import {
  chatMockSeed,
  MOCK_CHAT_ACTOR,
  MOCK_CHAT_HOST,
  type ChatMockScenario,
  type ChatMockSeed,
} from './chat-mock-scenarios';
/**
 * The team a mock store ID such as `team:eng` stands for: the fields the
 * native store ID would encode, which the reply scope names.
 */
function mockChatTeam(
  snapshot: AgentSnapshot | undefined,
  storeId: string,
): ChatTeamRef {
  const store = snapshot?.stores.find((s) => s.id === storeId);
  return {
    profile: store?.server ?? 'demo',
    accountAlias: store?.account ?? 'me',
    teamAlias: store?.kind === 'team' ? store.alias : storeId,
    teamId: store?.kind === 'team' ? store.team_id_hex : '03' + 'ab'.repeat(32),
  };
}
/**
 * A preview's text as the agent reports it: the first line, whole when it
 * fits in `CHAT_SNIPPET_BYTES` UTF-8 bytes, otherwise cut on a character
 * boundary to leave room for an ellipsis.
 */
function previewText(text: string): string {
  const line = text.split(/[\r\n]/, 1)[0];
  const utf8 = (c: string) => {
    const point = c.codePointAt(0)!;
    return point < 0x80 ? 1 : point < 0x800 ? 2 : point < 0x10000 ? 3 : 4;
  };
  let bytes = 0;
  let cut = '';
  for (const c of line) {
    bytes += utf8(c);
    if (bytes > CHAT_SNIPPET_BYTES) return `${cut}…`;
    // The ellipsis takes three bytes.
    if (bytes <= CHAT_SNIPPET_BYTES - 3) cut += c;
  }
  return line;
}
/** A stored submission's input, which a repeated prepare must match. */
const messageInput = (submission: string, channel: string, text: string) =>
  JSON.stringify({ action: 'prepare-message', submission, channel, text });
/**
 * In-memory demo only; native chat never writes browser storage. Each team
 * starts in the state `scenario` seeds (`chat-mock-scenarios.ts`).
 */
export function mockChat(
  snapshot?: AgentSnapshot,
  scenario: ChatMockScenario = 'default',
) {
  const accounts = new Map<
    string,
    { version: bigint; waiters: Set<() => void>; polling: boolean }
  >();
  const views = new Map<string, Set<() => void>>();
  const accountFor = ({ profile, accountAlias }: ChatTeamRef) => {
    const key = JSON.stringify([profile, accountAlias]);
    let account = accounts.get(key);
    if (!account) {
      account = { version: 1n, waiters: new Set(), polling: false };
      accounts.set(key, account);
    }
    return account;
  };
  const teams = new Map<
    string,
    {
      channels: ChatChannel[];
      messages: Map<string, ChatMessage[]>;
      operations: Map<string, ChatOperation>;
      submissions: Map<string, { input: string; op: ChatOperation }>;
      versions: Map<string, bigint>;
      reads: Map<string, bigint>;
      waiters: Set<() => void>;
      seed: ChatMockSeed;
    }
  >();
  const intents = new Map<string, { submission: string; text: string }>();
  const completionMarkers = new Set<string>();
  let counter = 10;
  const chat = async (
    storeId: string,
    action: ChatAction,
    viewId = '',
  ): Promise<ChatReply> => {
    const ref = mockChatTeam(snapshot, storeId);
    const account = accountFor(ref);
    let team = teams.get(storeId);
    if (!team) {
      const seed = chatMockSeed(scenario, snapshot, storeId);
      // Each conversation has its own inbox version, none past the head.
      const versions = new Map(
        seed.channels.map((c, i) => [c.id, account.version + BigInt(i)]),
      );
      if (seed.channels.length > 1) {
        account.version += BigInt(seed.channels.length - 1);
        for (const wake of account.waiters) wake();
        account.waiters.clear();
      }
      team = {
        channels: seed.channels,
        messages: seed.messages,
        operations: new Map(
          seed.operations.map(({ operation }) => [operation.id, operation]),
        ),
        submissions: new Map(
          seed.operations.map(({ operation, submission, text }) => [
            submission,
            {
              input: messageInput(submission, operation.channel, text),
              op: operation,
            },
          ]),
        ),
        versions,
        reads: seed.reads,
        waiters: new Set(),
        seed,
      };
      teams.set(storeId, team);
    }
    const bump = (channel: string) => {
      account.version += 1n;
      team.versions.set(channel, account.version);
      for (const wake of account.waiters) wake();
      account.waiters.clear();
    };
    const sendMessage = (op: ChatOperation, text: string) => {
      op.state = 'uncertain';
      const rows = team.messages.get(op.channel) ?? [];
      const sequence = String(BigInt(rows.at(-1)?.sequence ?? '0') + 1n);
      rows.push({
        id: op.id,
        sequence,
        sender: MOCK_CHAT_ACTOR,
        send_time: String(Date.now()),
        insert_time: String(Date.now()),
        content: { kind: 'text', text },
      });
      team.messages.set(op.channel, rows);
      team.reads.set(op.channel, BigInt(sequence));
      bump(op.channel);
      op.confirmation = { kind: 'message-sent', sequence };
      op.state = 'confirmed';
    };
    let result: ChatResult;
    if (
      action.action === 'load-intent' ||
      action.action === 'save-intent' ||
      action.action === 'clear-intent' ||
      action.action === 'import-intent'
    ) {
      if (action.host !== MOCK_CHAT_HOST || action.actor !== MOCK_CHAT_ACTOR)
        throw new Error('Saved message scope changed.');
      if (
        action.action === 'load-intent' &&
        team.seed.intentFailures.has(action.channel)
      )
        // The agent's catch-all reply when its saved-message store is invalid.
        throw {
          code: 'operation-failed',
          message:
            'FOKS application configuration is invalid: saved chat state is invalid',
          fatal: false,
          retryable: false,
          ambiguous: false,
        };
      const key = JSON.stringify([
        storeId,
        action.host,
        action.actor,
        action.channel,
      ]);
      const saved = intents.get(key);
      if (
        action.action === 'save-intent' ||
        action.action === 'import-intent'
      ) {
        if (!(
          action.action === 'import-intent' &&
          completionMarkers.has(action.source)
        )) {
          if (
            saved &&
            (saved.submission !== action.submission ||
              saved.text !== action.text)
          )
            throw new Error(
              'Another message is already saved for this channel.',
            );
          if (!saved && intents.size >= 128)
            throw new Error('Saved message capacity reached.');
          intents.set(key, {
            submission: action.submission,
            text: action.text,
          });
          if (action.action === 'import-intent')
            completionMarkers.add(action.source);
        }
      } else if (action.action === 'clear-intent') {
        if (saved && saved.submission !== action.submission)
          throw new Error('Saved message changed.');
        intents.delete(key);
      }
      result = {
        kind: 'intent',
        channel: action.channel,
        intent:
          action.action === 'clear-intent' || action.action === 'import-intent'
            ? null
            : structuredClone(intents.get(key) ?? null),
      };
    } else if (action.action === 'channels')
      result = {
        kind: 'channels',
        channels: [...team.channels],
        version: String(team.channels.length),
      };
    else if (
      action.action === 'history' ||
      action.action === 'notification-history'
    ) {
      const all = team.messages.get(action.channel) ?? [];
      const after = action.action === 'history' ? action.after : undefined;
      const newer = all.filter(
        (m) => after === undefined || BigInt(m.sequence) > BigInt(after),
      );
      // As the agent reads it: `before` names the range of a page's width
      // that ends one below it, whatever of that range exists.
      const rows = newer
        .filter(
          (m) =>
            action.before === null ||
            (BigInt(m.sequence) < BigInt(action.before) &&
              BigInt(m.sequence) >=
                BigInt(action.before) - BigInt(CHAT_PAGE_ROWS)),
        )
        .slice(-CHAT_PAGE_ROWS);
      result = {
        kind: 'history',
        ...(after === undefined
          ? {}
          : {
              gap:
                newer.length > CHAT_PAGE_ROWS ||
                BigInt(all.at(-1)?.sequence ?? '0') < BigInt(after),
            }),
        channel: action.channel,
        // The agent cuts each notification text to 256 characters.
        messages: (after === undefined ? rows : [...rows].reverse()).map((m) =>
          action.action === 'notification-history' && m.content.kind === 'text'
            ? {
                ...m,
                content: {
                  kind: 'text',
                  text: Array.from(m.content.text).slice(0, 256).join(''),
                },
              }
            : m,
        ),
        before:
          rows.length && rows[0].sequence !== '1' ? rows[0].sequence : null,
        missing_predecessors: [
          ...new Set(
            rows.flatMap((m) => team.seed.missingPredecessors.get(m.id) ?? []),
          ),
        ],
      };
    } else if (action.action === 'inbox' || action.action === 'sync-inbox') {
      const blockedChannels = team.channels
        .map((c) => c.id)
        .filter(
          (id) =>
            team.seed.blocked.has(id) ||
            (action.action === 'sync-inbox' &&
              action.blocked_channels?.includes(id)),
        );
      result = {
        kind: 'inbox',
        channels: [...team.channels],
        read_retry_pending: false,
        previews_incomplete: false,
        blocked_channels: blockedChannels,
        // A degraded projection's cursor stops short of the head.
        cursor: String(account.version - (team.seed.degraded ? 1n : 0n)),
        head: String(account.version),
        degraded: team.seed.degraded,
        conversations: team.channels.map((channel) => {
          const rows = team.messages.get(channel.id) ?? [];
          const blocked = blockedChannels.includes(channel.id);
          const last = rows.at(-1);
          const readThrough = team.reads.get(channel.id) ?? 0n;
          const lastSequence = last ? BigInt(last.sequence) : 0n;
          return {
            channel,
            inbox_version: String(team.versions.get(channel.id) ?? 1n),
            read_through: String(readThrough),
            pending_read: null,
            unread: String(
              lastSequence > readThrough ? lastSequence - readThrough : 0n,
            ),
            hidden: team.seed.hidden.has(channel.id),
            muted: team.seed.muted.has(channel.id),
            preview:
              last && !blocked
                ? {
                    sender: last.sender,
                    send_time: last.send_time,
                    insert_time: last.insert_time,
                    content:
                      last.content.kind === 'text'
                        ? { kind: 'text', text: previewText(last.content.text) }
                        : last.content,
                  }
                : null,
          };
        }),
      };
    } else if (action.action === 'mark-read') {
      const sequence = BigInt(action.sequence);
      const old = team.reads.get(action.channel) ?? 0n;
      if (sequence > old) {
        team.reads.set(action.channel, sequence);
        bump(action.channel);
      }
      result = {
        kind: 'read',
        channel: action.channel,
        sequence: action.sequence,
      };
    } else if (action.action === 'poll-inbox') {
      if (account.polling)
        throw {
          code: 'busy',
          message: 'Account already polling.',
          fatal: false,
          retryable: true,
          ambiguous: false,
        };
      account.polling = true;
      const since = BigInt(action.since);
      try {
        if (account.version <= since) {
          await new Promise<void>((resolve, reject) => {
            const cleanup = () => {
              clearTimeout(timer);
              account.waiters.delete(wake);
              views.get(viewId)?.delete(cancel);
              if (!views.get(viewId)?.size) views.delete(viewId);
            };
            const wake = () => {
              cleanup();
              resolve();
            };
            const cancel = () => {
              cleanup();
              reject(cancelled());
            };
            const timer = setTimeout(
              wake,
              Math.min(action.timeout_milliseconds || 250, 250),
            );
            account.waiters.add(wake);
            const owned = views.get(viewId) ?? new Set();
            owned.add(cancel);
            views.set(viewId, owned);
          });
        }
        result = {
          kind: 'poll',
          bumped: account.version > since,
          inbox_version: String(account.version),
        };
      } finally {
        account.polling = false;
      }
    } else if (action.action === 'pending')
      result = {
        kind: 'pending',
        operations: [...team.operations.values()].filter(
          (o) => o.state === 'prepared' || o.state === 'uncertain',
        ),
      };
    else if (action.action === 'cleanup-pending')
      result = { kind: 'cleanup-pending', operations: [] };
    else if (
      action.action === 'prepare-channel' ||
      action.action === 'prepare-message' ||
      action.action === 'submit-message'
    ) {
      const old = team.submissions.get(action.submission);
      const input =
        action.action === 'prepare-channel'
          ? JSON.stringify(action)
          : messageInput(action.submission, action.channel, action.text);
      if (old && old.input !== input)
        throw { code: 'conflict', message: 'Submission changed.' };
      const id = old?.op.id ?? (++counter).toString(16).padStart(32, '0');
      const op: ChatOperation = old?.op ?? {
        id,
        channel: action.action === 'prepare-channel' ? id : action.channel,
        kind:
          action.action === 'prepare-channel'
            ? 'create-channel'
            : 'send-message',
        state: 'prepared',
        confirmation: null,
        rejection_code: null,
      };
      team.operations.set(op.id, op);
      team.submissions.set(action.submission, { input, op });
      if (!old && action.action === 'submit-message')
        sendMessage(op, action.text);
      result = { kind: 'operation', operation: { ...op } };
    } else if (action.action === 'operation-body') {
      const op = team.operations.get(action.operation);
      if (!op || op.channel !== action.channel || op.kind !== 'send-message')
        throw { code: 'chat-not-found', message: 'Operation not found.' };
      const storedSubmission = [...team.submissions.values()].find(
        (s) => s.op.id === op.id,
      );
      const submitted = storedSubmission
        ? (JSON.parse(storedSubmission.input) as ChatAction)
        : null;
      result = {
        kind: 'operation-body',
        operation: op.id,
        channel: op.channel,
        text:
          ['prepared', 'uncertain'].includes(op.state) &&
          submitted?.action === 'prepare-message'
            ? submitted.text
            : null,
      };
    } else {
      const op = team.operations.get(action.operation);
      if (!op)
        throw { code: 'chat-not-found', message: 'Operation not found.' };
      if (action.action === 'cancel') {
        if (op.state !== 'prepared' && op.state !== 'cancelled')
          throw {
            code: 'chat-operation-state',
            message: 'Only unsent messages can be cancelled.',
          };
        op.state = 'cancelled';
      } else if (action.action === 'reconcile' && op.state === 'uncertain') {
        const message = team.messages
          .get(op.channel)
          ?.find((m) => m.id === op.id);
        if (message) {
          op.state = 'confirmed';
          op.confirmation = {
            kind: 'message-sent',
            sequence: message.sequence,
          };
        } else if (
          op.kind === 'create-channel' &&
          team.channels.some((c) => c.id === op.channel)
        ) {
          op.state = 'confirmed';
          op.confirmation = { kind: 'channel-created' };
        }
      } else if (action.action === 'attempt' && op.state === 'prepared') {
        const input = [...team.submissions.values()].find(
          (s) => s.op === op,
        )!.input;
        const submitted = JSON.parse(input) as ChatAction;
        // As the agent does, a host rejection is recorded in the ledger
        // before anything is applied, and the attempt fails; a later status
        // read reports the code.
        const rejection = team.seed.rejections.get(op.id);
        if (rejection !== undefined) {
          op.state = 'rejected';
          op.rejection_code = rejection;
          throw {
            code: 'operation-failed',
            message: `The server rejected this message (status ${rejection}).`,
            fatal: false,
            retryable: false,
            ambiguous: false,
          };
        }
        if (submitted.action === 'prepare-channel') {
          op.confirmation = { kind: 'channel-created' };
          team.channels.push({
            id: op.channel,
            name: submitted.name,
            description: submitted.description
              ? Array.from(
                  submitted.description,
                  (c) => Array.from(c.toLowerCase())[0],
                ).join('')
              : null,
            admin: submitted.admin,
            readable: true,
            writable: true,
            read_role: submitted.admin ? 'Admin' : 'Member (0)',
            write_role: submitted.admin ? 'Admin' : 'Member (0)',
          });
          team.messages.set(op.channel, []);
          team.reads.set(op.channel, 0n);
          bump(op.channel);
        }
        if (submitted.action === 'prepare-message')
          sendMessage(op, submitted.text);
        op.state = 'confirmed';
      }
      result = { kind: 'operation', operation: { ...op } };
    }
    // As the native bridge does, every reply passes the IPC decoder, here on
    // a copy serialized as IPC would carry it.
    const reply: unknown = JSON.parse(
      JSON.stringify({
        scope: {
          store: {
            profile: ref.profile,
            account_alias: ref.accountAlias,
            team_alias: ref.teamAlias,
            team_id: ref.teamId,
          },
          host: MOCK_CHAT_HOST,
          actor: MOCK_CHAT_ACTOR,
        },
        result,
      }),
    );
    return decodeChatReplyFor(reply, ref, action);
  };
  return Object.assign(chat, {
    cancel: async (view: string) => {
      for (const cancel of [...(views.get(view) ?? [])]) cancel();
    },
    /**
     * Sets a stored ledger operation's outcome, for a test that needs a state
     * the mock reaches no other way. Replies are decoded copies, so changing
     * one leaves the mock's state as it was.
     */
    updateOperation: (
      storeId: string,
      id: string,
      change: Partial<
        Pick<ChatOperation, 'state' | 'confirmation' | 'rejection_code'>
      >,
    ) => {
      const op = teams.get(storeId)?.operations.get(id);
      if (!op) throw new Error('No such mock chat operation.');
      // The decoder checks the operation on every reply naming it.
      try {
        decodeChatOperation(JSON.parse(JSON.stringify({ ...op, ...change })));
      } catch {
        throw new Error('The chat decoder refuses this operation outcome.');
      }
      Object.assign(op, change);
    },
  });
}
/** A mock chat, which the mock bridge also exposes as its `chat`. */
export type MockChat = ReturnType<typeof mockChat>;
/** The mock chat behind a bridge's `chat`, which must be the mock bridge's. */
export function mockChatOf(bridge: { chat: unknown }): MockChat {
  const chat = bridge.chat;
  if (typeof chat !== 'function' || !('updateOperation' in chat))
    throw new Error('The bridge chat is not the mock chat.');
  return chat as MockChat;
}
