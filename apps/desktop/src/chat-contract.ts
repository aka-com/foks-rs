/** The chat IPC contract. Validation failures never quote returned plaintext. */
import {
  CHAT_TEXT_BYTES,
  CHAT_PAGE_ROWS,
  CHAT_CHANNEL_ROWS,
  CHAT_NAME_BYTES,
  CHAT_LABEL_BYTES,
  CHAT_MISSING_PREDECESSORS,
  CHAT_PENDING_ROWS,
} from './chat-limits';
export { CHAT_TEXT_BYTES } from './chat-limits';
export type ChatAction =
  | { action: 'channels' }
  | { action: 'pending' }
  | { action: 'history'; channel: string; before: string | null }
  | {
      action: 'prepare-channel';
      submission: string;
      name: string;
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
  admin: boolean;
  readable: boolean;
  read_role: string;
  write_role: string;
}
export interface ChatMessage {
  id: string;
  sequence: string;
  sender: string | null;
  content:
    { kind: 'text'; text: string } | { kind: 'unsupported' | 'oversized' };
}
export interface ChatOperation {
  id: string;
  channel: string;
  create: boolean;
  state: 'prepared' | 'uncertain' | 'confirmed' | 'rejected' | 'cancelled';
  sequence: string | null;
  rejection_code: number | null;
}
export type ChatResult =
  | { kind: 'channels'; channels: ChatChannel[]; version: string }
  | {
      kind: 'history';
      channel: string;
      messages: ChatMessage[];
      before: string | null;
      missing_predecessors: string[];
    }
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
function operation(value: unknown): ChatOperation {
  const v = object(value, [
    'id',
    'channel',
    'create',
    'state',
    'sequence',
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
    create: bool(v.create),
    state: state as ChatOperation['state'],
    sequence: v.sequence === null ? null : sequence(v.sequence),
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
    (state === 'confirmed' && !op.create) !== (op.sequence !== null) ||
    op.sequence === '0'
  )
    return fail();
  return op;
}
export function decodeChatReply(
  value: unknown,
  storeId: string,
  action: ChatAction,
): ChatReply {
  const v = object(value, ['scope', 'result']);
  const scope = object(v.scope, ['store', 'host', 'actor']);
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
  const r = object(v.result);
  let result: ChatResult;
  if (r.kind === 'channels' && action.action === 'channels') {
    object(r, ['kind', 'channels', 'version']);
    const channels = array(r.channels, CHAT_CHANNEL_ROWS, (v): ChatChannel => {
      const c = object(v, [
        'id',
        'name',
        'admin',
        'readable',
        'read_role',
        'write_role',
      ]);
      return {
        id: chatId(c.id),
        name: text(c.name, CHAT_NAME_BYTES),
        admin: bool(c.admin),
        readable: bool(c.readable),
        read_role: text(c.read_role),
        write_role: text(c.write_role),
      };
    });
    if (new Set(channels.map((c) => c.id)).size !== channels.length)
      return fail();
    result = { kind: 'channels', channels, version: sequence(r.version) };
  } else if (r.kind === 'history' && action.action === 'history') {
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
      const m = object(value, ['id', 'sequence', 'sender', 'content']);
      const c = object(m.content);
      object(c, c.kind === 'text' ? ['kind', 'text'] : ['kind']);
      const content: ChatMessage['content'] =
        c.kind === 'text'
          ? { kind: 'text', text: text(c.text, CHAT_TEXT_BYTES) }
          : c.kind === 'unsupported' || c.kind === 'oversized'
            ? { kind: c.kind }
            : fail();
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
        content,
      };
    });
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
        (op.create || op.channel !== action.channel)) ||
      (action.action === 'prepare-channel' && !op.create)
    )
      return fail();
    result = { kind: 'operation', operation: op };
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
