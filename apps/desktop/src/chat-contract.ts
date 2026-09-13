/** The chat IPC contract. Validation failures never quote returned plaintext. */
import {
  CHAT_TEXT_BYTES,
  CHAT_PAGE_ROWS,
  CHAT_CHANNEL_ROWS,
  CHAT_NAME_BYTES,
  CHAT_DESCRIPTION_BYTES,
  CHAT_SNIPPET_BYTES,
  CHAT_LABEL_BYTES,
  CHAT_MISSING_PREDECESSORS,
  CHAT_PENDING_ROWS,
  CHAT_INBOX_ROWS,
  CHAT_POLL_MILLISECONDS,
} from './chat-limits';
export { CHAT_TEXT_BYTES } from './chat-limits';
export type ChatAction =
  | { action: 'operation-body'; operation: string; channel: string }
  | { action: 'channels' }
  | { action: 'pending' }
  | { action: 'inbox' }
  | { action: 'sync-inbox'; blocked_channels?: string[] }
  | { action: 'mark-read'; channel: string; sequence: string }
  | { action: 'poll-inbox'; since: string; timeout_milliseconds: number }
  | { action: 'history'; channel: string; before: string | null }
  | { action: 'notification-history'; channel: string; before: string | null }
  | {
      action: 'prepare-channel';
      submission: string;
      name: string;
      description: string;
      admin: boolean;
    }
  | {
      action: 'prepare-message';
      submission: string;
      channel: string;
      text: string;
    }
  | { action: 'attempt' | 'cancel' | 'finalize' | 'status'; operation: string };
export interface ChatScope {
  store: {
    profile: string;
    account_alias: string;
    team_alias: string;
    team_id: string;
  };
  host: string;
  actor: string;
}
export interface ChatChannel {
  id: string;
  name: string;
  description: string | null;
  admin: boolean;
  readable: boolean;
  writable: boolean;
  read_role: string;
  write_role: string;
}
export interface ChatMessage {
  id: string;
  sequence: string;
  sender: string | null;
  send_time: string;
  insert_time: string;
  content:
    { kind: 'text'; text: string } | { kind: 'unsupported' | 'oversized' };
}
export interface ChatPreview {
  sender: string | null;
  send_time: string;
  insert_time: string;
  content:
    { kind: 'text'; text: string } | { kind: 'unsupported' | 'oversized' };
}
export interface ChatConversation {
  channel: ChatChannel;
  inbox_version: string;
  read_through: string;
  pending_read: string | null;
  unread: string;
  hidden: boolean;
  muted: boolean;
  preview: ChatPreview | null;
}
export interface ChatOperation {
  id: string;
  channel: string;
  kind: 'create-channel' | 'send-message';
  state: 'prepared' | 'uncertain' | 'confirmed' | 'rejected' | 'cancelled';
  receipt:
    | { kind: 'channel-created' }
    | { kind: 'message-sent'; sequence: string }
    | null;
  rejection_code: number | null;
}
export type ChatResult =
  | {
      kind: 'operation-body';
      operation: string;
      channel: string;
      text: string | null;
    }
  | { kind: 'channels'; channels: ChatChannel[]; version: string }
  | {
      kind: 'history';
      channel: string;
      messages: ChatMessage[];
      before: string | null;
      missing_predecessors: string[];
    }
  | {
      kind: 'inbox';
      channels: ChatChannel[];
      read_retry_pending: boolean;
      previews_incomplete: boolean;
      blocked_channels: string[];
      cursor: string;
      head: string;
      degraded: boolean;
      conversations: ChatConversation[];
    }
  | { kind: 'read'; channel: string; sequence: string }
  | { kind: 'poll'; bumped: boolean; inbox_version: string }
  | { kind: 'operation'; operation: ChatOperation }
  | { kind: 'pending'; operations: ChatOperation[] };
export interface ChatReply {
  scope: ChatScope;
  result: ChatResult;
}
const fail = (): never => {
  throw {
    code: 'chat-integrity',
    message: 'The chat response is invalid or belongs to another account.',
    fatal: true,
    retryable: false,
    ambiguous: false,
  };
};
function object(
  v: unknown,
  fields?: readonly string[],
): Record<string, unknown> {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return fail();
  if (fields && Object.keys(v).some((key) => !fields.includes(key)))
    return fail();
  return v as Record<string, unknown>;
}
function text(v: unknown, max = CHAT_LABEL_BYTES): string {
  if (typeof v !== 'string' || new TextEncoder().encode(v).length > max)
    return fail();
  return v;
}
function bool(v: unknown): boolean {
  if (typeof v !== 'boolean') return fail();
  return v;
}
export function chatId(v: unknown): string {
  const s = text(v, 32);
  if (!/^[0-9a-f]{32}$/.test(s) || /^0+$/.test(s)) return fail();
  return s;
}
export function sequence(v: unknown): string {
  const s = text(v, 19);
  if (!/^(0|[1-9][0-9]*)$/.test(s) || BigInt(s) > 9223372036854775807n)
    return fail();
  return s;
}
function entity(v: unknown, prefix?: string): string {
  const s = text(v, 66);
  if (!/^[0-9a-f]{66}$/.test(s) || (prefix && !s.startsWith(prefix)))
    return fail();
  return s;
}
function array<T>(v: unknown, max: number, decode: (v: unknown) => T): T[] {
  if (!Array.isArray(v) || v.length > max) return fail();
  return v.map(decode);
}
function channel(value: unknown): ChatChannel {
  const c = object(value, [
    'id',
    'name',
    'description',
    'admin',
    'readable',
    'writable',
    'read_role',
    'write_role',
  ]);
  if (bool(c.writable) && !bool(c.readable)) return fail();
  return {
    id: chatId(c.id),
    name: text(c.name, CHAT_NAME_BYTES),
    description:
      c.description === null
        ? null
        : text(c.description, CHAT_DESCRIPTION_BYTES),
    admin: bool(c.admin),
    readable: bool(c.readable),
    writable: bool(c.writable),
    read_role: text(c.read_role),
    write_role: text(c.write_role),
  };
}
function content(value: unknown, maximum: number): ChatMessage['content'] {
  const c = object(value);
  object(c, c.kind === 'text' ? ['kind', 'text'] : ['kind']);
  return c.kind === 'text'
    ? { kind: 'text', text: text(c.text, maximum) }
    : c.kind === 'unsupported' || c.kind === 'oversized'
      ? { kind: c.kind }
      : fail();
}
function operation(value: unknown): ChatOperation {
  const v = object(value, [
    'id',
    'channel',
    'kind',
    'state',
    'receipt',
    'rejection_code',
  ]);
  const state = text(v.state);
  if (
    !['prepared', 'uncertain', 'confirmed', 'rejected', 'cancelled'].includes(
      state,
    )
  )
    return fail();
  const op: ChatOperation = {
    id: chatId(v.id),
    channel: chatId(v.channel),
    kind:
      v.kind === 'create-channel' || v.kind === 'send-message'
        ? v.kind
        : fail(),
    state: state as ChatOperation['state'],
    receipt: operationReceipt(v.receipt),
    rejection_code:
      v.rejection_code === null
        ? null
        : typeof v.rejection_code === 'number' &&
            Number.isSafeInteger(v.rejection_code)
          ? v.rejection_code
          : fail(),
  };
  if (
    (state === 'rejected') !== (op.rejection_code !== null) ||
    (state === 'confirmed') !== (op.receipt !== null) ||
    (op.receipt !== null &&
      (op.kind === 'create-channel') !==
        (op.receipt.kind === 'channel-created'))
  )
    return fail();
  return op;
}
function operationReceipt(value: unknown): ChatOperation['receipt'] {
  if (value === null) return null;
  if (typeof value !== 'object' || !value) return fail();
  const kind = (value as Record<string, unknown>).kind;
  if (kind === 'channel-created') {
    object(value, ['kind']);
    return { kind };
  }
  if (kind === 'message-sent') {
    const v = object(value, ['kind', 'sequence']);
    const position = sequence(v.sequence);
    if (position === '0') return fail();
    return { kind, sequence: position };
  }
  return fail();
}
export function decodeChatScope(value: unknown, storeId: string): ChatScope {
  const scope = object(value, ['store', 'host', 'actor']);
  const store = object(scope.store, [
    'profile',
    'account_alias',
    'team_alias',
    'team_id',
  ]);
  let expected: Record<string, unknown>;
  try {
    expected = object(JSON.parse(storeId));
  } catch {
    return fail();
  }
  if (
    expected.kind !== 'team' ||
    expected.profile !== store.profile ||
    expected.accountAlias !== store.account_alias ||
    expected.teamAlias !== store.team_alias ||
    expected.teamId !== store.team_id
  )
    return fail();
  const resolved: ChatScope = {
    store: {
      profile: text(store.profile),
      account_alias: text(store.account_alias),
      team_alias: text(store.team_alias),
      team_id: entity(store.team_id, '03'),
    },
    host: entity(scope.host, '02'),
    actor: entity(scope.actor, '01'),
  };
  return resolved;
}
export function decodeChatReply(
  value: unknown,
  storeId: string,
  action: ChatAction,
): ChatReply {
  const v = object(value, ['scope', 'result']);
  const resolved = decodeChatScope(v.scope, storeId);
  const r = object(v.result);
  let result: ChatResult;
  if (r.kind === 'channels' && action.action === 'channels') {
    object(r, ['kind', 'channels', 'version']);
    const channels = array(r.channels, CHAT_CHANNEL_ROWS, channel);
    if (new Set(channels.map((c) => c.id)).size !== channels.length)
      return fail();
    result = { kind: 'channels', channels, version: sequence(r.version) };
  } else if (
    r.kind === 'history' &&
    (action.action === 'history' || action.action === 'notification-history')
  ) {
    object(r, [
      'kind',
      'channel',
      'messages',
      'before',
      'missing_predecessors',
    ]);
    const channel = chatId(r.channel);
    if (channel !== action.channel) return fail();
    const messages = array(r.messages, CHAT_PAGE_ROWS, (value): ChatMessage => {
      const m = object(value, [
        'id',
        'sequence',
        'sender',
        'send_time',
        'insert_time',
        'content',
      ]);
      const seq = sequence(m.sequence);
      if (
        seq === '0' ||
        (action.before !== null && BigInt(seq) >= BigInt(action.before))
      )
        return fail();
      return {
        id: chatId(m.id),
        sequence: seq,
        sender: m.sender === null ? null : entity(m.sender, '01'),
        send_time: sequence(m.send_time),
        insert_time: sequence(m.insert_time),
        content: content(m.content, CHAT_TEXT_BYTES),
      };
    });
    if (
      action.action === 'notification-history' &&
      messages.some(
        (m) =>
          m.content.kind === 'text' && Array.from(m.content.text).length > 256,
      )
    )
      fail();
    if (
      new Set(messages.map((m) => m.id)).size !== messages.length ||
      new Set(messages.map((m) => m.sequence)).size !== messages.length
    )
      return fail();
    const before = r.before === null ? null : sequence(r.before);
    const min = messages.reduce<bigint | null>(
      (min, m) =>
        min === null || BigInt(m.sequence) < min ? BigInt(m.sequence) : min,
      null,
    );
    if (before !== (min !== null && min > 1n ? String(min) : null))
      return fail();
    result = {
      kind: 'history',
      channel,
      messages,
      before,
      missing_predecessors: array(
        r.missing_predecessors,
        CHAT_MISSING_PREDECESSORS,
        (value) => {
          const n = sequence(value);
          return n === '0' ? fail() : n;
        },
      ),
    };
  } else if (
    r.kind === 'inbox' &&
    (action.action === 'inbox' || action.action === 'sync-inbox')
  ) {
    object(r, [
      'kind',
      'cursor',
      'head',
      'degraded',
      'conversations',
      'channels',
      'read_retry_pending',
      'previews_incomplete',
      'blocked_channels',
    ]);
    const cursor = sequence(r.cursor);
    const head = sequence(r.head);
    const degraded = bool(r.degraded);
    if (BigInt(cursor) > BigInt(head) || degraded !== (cursor !== head))
      return fail();
    const conversations = array(
      r.conversations,
      CHAT_INBOX_ROWS,
      (value): ChatConversation => {
        const c = object(value, [
          'channel',
          'inbox_version',
          'read_through',
          'pending_read',
          'unread',
          'hidden',
          'muted',
          'preview',
        ]);
        const inboxVersion = sequence(c.inbox_version);
        const pendingRead =
          c.pending_read === null ? null : sequence(c.pending_read);
        const preview =
          c.preview === null
            ? null
            : (() => {
                const p = object(c.preview, [
                  'sender',
                  'send_time',
                  'insert_time',
                  'content',
                ]);
                return {
                  sender: p.sender === null ? null : entity(p.sender, '01'),
                  send_time: sequence(p.send_time),
                  insert_time: sequence(p.insert_time),
                  content: content(p.content, CHAT_SNIPPET_BYTES),
                };
              })();
        const decoded = {
          channel: channel(c.channel),
          inbox_version: inboxVersion,
          read_through: sequence(c.read_through),
          pending_read: pendingRead,
          unread: sequence(c.unread),
          hidden: bool(c.hidden),
          muted: bool(c.muted),
          preview,
        };
        if (
          inboxVersion === '0' ||
          BigInt(inboxVersion) > BigInt(head) ||
          pendingRead === '0' ||
          !decoded.channel.readable
        )
          return fail();
        return decoded;
      },
    );
    if (
      new Set(conversations.map((c) => c.channel.id)).size !==
        conversations.length ||
      new Set(conversations.map((c) => c.inbox_version)).size !==
        conversations.length
    )
      return fail();
    const discovered = array(r.channels, CHAT_CHANNEL_ROWS, channel);
    if (new Set(discovered.map((c) => c.id)).size !== discovered.length)
      return fail();
    const blocked = array(r.blocked_channels, CHAT_CHANNEL_ROWS, chatId);
    if (
      new Set(blocked).size !== blocked.length ||
      blocked.some((id) => !discovered.some((c) => c.id === id)) ||
      conversations.some(
        (c) => blocked.includes(c.channel.id) && c.preview !== null,
      )
    )
      return fail();
    result = {
      kind: 'inbox',
      blocked_channels: blocked,
      cursor,
      head,
      degraded,
      conversations,
      channels: discovered,
      read_retry_pending: bool(r.read_retry_pending),
      previews_incomplete: bool(r.previews_incomplete),
    };
  } else if (r.kind === 'read' && action.action === 'mark-read') {
    object(r, ['kind', 'channel', 'sequence']);
    const channel = chatId(r.channel);
    const readThrough = sequence(r.sequence);
    if (
      channel !== action.channel ||
      readThrough === '0' ||
      readThrough !== action.sequence
    )
      return fail();
    result = { kind: 'read', channel, sequence: readThrough };
  } else if (r.kind === 'poll' && action.action === 'poll-inbox') {
    object(r, ['kind', 'bumped', 'inbox_version']);
    if (
      !Number.isSafeInteger(action.timeout_milliseconds) ||
      action.timeout_milliseconds < 0 ||
      action.timeout_milliseconds > CHAT_POLL_MILLISECONDS
    )
      return fail();
    const since = sequence(action.since);
    const inboxVersion = sequence(r.inbox_version);
    const bumped = bool(r.bumped);
    if (bumped !== BigInt(inboxVersion) > BigInt(since)) return fail();
    result = { kind: 'poll', bumped, inbox_version: inboxVersion };
  } else if (
    r.kind === 'operation' &&
    [
      'prepare-channel',
      'prepare-message',
      'attempt',
      'cancel',
      'finalize',
      'status',
    ].includes(action.action)
  ) {
    object(r, ['kind', 'operation']);
    const op = operation(r.operation);
    if (
      ('operation' in action && action.operation !== op.id) ||
      (action.action === 'prepare-message' &&
        (op.kind !== 'send-message' || op.channel !== action.channel)) ||
      (action.action === 'prepare-channel' && op.kind !== 'create-channel')
    )
      return fail();
    result = { kind: 'operation', operation: op };
  } else if (
    r.kind === 'operation-body' &&
    action.action === 'operation-body'
  ) {
    object(r, ['kind', 'operation', 'channel', 'text']);
    const operationId = chatId(r.operation);
    const channelId = chatId(r.channel);
    if (operationId !== action.operation || channelId !== action.channel)
      return fail();
    result = {
      kind: 'operation-body',
      operation: operationId,
      channel: channelId,
      text: r.text === null ? null : text(r.text, CHAT_TEXT_BYTES),
    };
  } else if (r.kind === 'pending' && action.action === 'pending') {
    object(r, ['kind', 'operations']);
    const operations = array(r.operations, CHAT_PENDING_ROWS, operation);
    if (
      new Set(operations.map((o) => o.id)).size !== operations.length ||
      operations.some((o) => !['prepared', 'uncertain'].includes(o.state))
    )
      return fail();
    result = { kind: 'pending', operations };
  } else return fail();
  return { scope: resolved, result };
}
