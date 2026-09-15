import type { AgentSnapshot } from './model';
import { cancelled } from './chat/client';
import { CHAT_PAGE_ROWS } from './chat-limits';
import type {
  ChatAction,
  ChatChannel,
  ChatMessage,
  ChatOperation,
  ChatReply,
  ChatResult,
} from './chat-contract';
/** In-memory demo only; native chat never writes browser storage. */
export function mockChat(snapshot?: AgentSnapshot) {
  const accounts = new Map<
    string,
    { version: bigint; waiters: Set<() => void>; polling: boolean }
  >();
  const views = new Map<string, Set<() => void>>();
  const accountFor = (storeId: string) => {
    const store = snapshot?.stores.find((s) => s.id === storeId);
    const key = JSON.stringify([
      store?.server ?? 'demo',
      store?.account ?? 'me',
    ]);
    let account = accounts.get(key);
    if (!account) {
      account = { version: 1n, waiters: new Set(), polling: false };
      accounts.set(key, account);
    }
    return { account, store };
  };
  const teams = new Map<
    string,
    {
      channels: ChatChannel[];
      messages: Map<string, ChatMessage[]>;
      operations: Map<string, ChatOperation>;
      submissions: Map<string, { input: string; op: ChatOperation }>;
      inboxVersion: bigint;
      versions: Map<string, bigint>;
      reads: Map<string, bigint>;
      waiters: Set<() => void>;
    }
  >();
  let counter = 10;
  const chat = async (
    storeId: string,
    action: ChatAction,
    viewId = '',
  ): Promise<ChatReply> => {
    const { account, store } = accountFor(storeId);
    let team = teams.get(storeId);
    if (!team) {
      const id = 'ab'.repeat(16);
      team = {
        channels: [
          {
            id,
            name: '',
            description: 'A place for the whole team.',
            admin: false,
            readable: true,
            writable: true,
            read_role: 'Member (0)',
            write_role: 'Member (0)',
          },
        ],
        messages: new Map([
          [
            id,
            [
              {
                id: 'cd'.repeat(16),
                sequence: '1',
                sender: null,
                send_time: '1700000000000',
                insert_time: '1700000000001',
                content: { kind: 'text', text: 'Team chat is ready.' },
              },
            ],
          ],
        ]),
        operations: new Map(),
        submissions: new Map(),
        inboxVersion: 1n,
        versions: new Map([[id, 1n]]),
        reads: new Map([[id, 0n]]),
        waiters: new Set(),
      };
      teams.set(storeId, team);
    }
    const bump = (channel: string) => {
      account.version += 1n;
      team.inboxVersion = account.version;
      team.versions.set(channel, account.version);
      for (const wake of account.waiters) wake();
      account.waiters.clear();
    };
    let result: ChatResult;
    if (action.action === 'channels')
      result = {
        kind: 'channels',
        channels: [...team.channels],
        version: String(team.channels.length),
      };
    else if (
      action.action === 'history' ||
      action.action === 'notification-history'
    ) {
      const rows = (team.messages.get(action.channel) ?? [])
        .filter(
          (m) =>
            action.before === null ||
            BigInt(m.sequence) < BigInt(action.before),
        )
        .slice(-CHAT_PAGE_ROWS);
      result = {
        kind: 'history',
        channel: action.channel,
        messages: rows,
        before:
          rows.length && rows[0].sequence !== '1' ? rows[0].sequence : null,
        missing_predecessors: [],
      };
    } else if (action.action === 'inbox' || action.action === 'sync-inbox') {
      result = {
        kind: 'inbox',
        channels: [...team.channels],
        read_retry_pending: false,
        previews_incomplete: false,
        blocked_channels:
          action.action === 'sync-inbox'
            ? (action.blocked_channels ?? []).filter((id) =>
                team.channels.some((c) => c.id === id),
              )
            : [],
        cursor: String(account.version),
        head: String(account.version),
        degraded: false,
        conversations: team.channels.map((channel) => {
          const rows = team.messages.get(channel.id) ?? [];
          const blocked =
            action.action === 'sync-inbox' &&
            action.blocked_channels?.includes(channel.id);
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
            hidden: false,
            muted: false,
            preview:
              last && !blocked
                ? {
                    sender: last.sender,
                    send_time: last.send_time,
                    insert_time: last.insert_time,
                    content:
                      last.content.kind === 'text'
                        ? {
                            kind: 'text',
                            text: last.content.text
                              .split(/[\r\n]/, 1)[0]
                              .slice(0, 512),
                          }
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
    else if (
      action.action === 'prepare-channel' ||
      action.action === 'prepare-message'
    ) {
      const old = team.submissions.get(action.submission);
      const input = JSON.stringify(action);
      if (old && old.input !== input)
        throw { code: 'conflict', message: 'Submission changed.' };
      const id = (++counter).toString(16).padStart(32, '0');
      const op: ChatOperation = old?.op ?? {
        id,
        channel: action.action === 'prepare-channel' ? id : action.channel,
        kind:
          action.action === 'prepare-channel'
            ? 'create-channel'
            : 'send-message',
        state: 'prepared',
        receipt: null,
        rejection_code: null,
      };
      team.operations.set(op.id, op);
      team.submissions.set(action.submission, { input, op });
      result = { kind: 'operation', operation: { ...op } };
    } else if (action.action === 'operation-body') {
      const op = team.operations.get(action.operation);
      if (!op || op.channel !== action.channel || op.kind !== 'send-message')
        throw { code: 'chat-not-found', message: 'Operation not found.' };
      const material = [...team.submissions.values()].find(
        (s) => s.op.id === op.id,
      );
      const submitted = material
        ? (JSON.parse(material.input) as ChatAction)
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
      if (action.action === 'cancel') op.state = 'cancelled';
      else if (action.action === 'attempt' && op.state === 'prepared') {
        const input = [...team.submissions.values()].find(
          (s) => s.op === op,
        )!.input;
        const submitted = JSON.parse(input) as ChatAction;
        if (submitted.action === 'prepare-channel') {
          op.receipt = { kind: 'channel-created' };
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
        if (submitted.action === 'prepare-message') {
          const rows = team.messages.get(op.channel) ?? [];
          op.receipt = {
            kind: 'message-sent',
            sequence: String(rows.length + 1),
          };
          rows.push({
            id: op.id,
            sequence: op.receipt.sequence,
            sender: '01' + 'ab'.repeat(32),
            send_time: String(Date.now()),
            insert_time: String(Date.now()),
            content: { kind: 'text', text: submitted.text },
          });
          team.messages.set(op.channel, rows);
          team.reads.set(op.channel, BigInt(op.receipt.sequence));
          bump(op.channel);
        }
        op.state = 'confirmed';
      }
      result = { kind: 'operation', operation: { ...op } };
    }
    return {
      scope: {
        store: {
          profile: store?.server ?? 'demo',
          account_alias: store?.account ?? 'me',
          team_alias: store?.kind === 'team' ? store.alias : storeId,
          team_id:
            store?.kind === 'team' ? store.team_id_hex : '03' + 'ab'.repeat(32),
        },
        host: '02' + 'ab'.repeat(32),
        actor: '01' + 'ab'.repeat(32),
      },
      result,
    };
  };
  return Object.assign(chat, {
    cancel: async (view: string) => {
      for (const cancel of [...(views.get(view) ?? [])]) cancel();
    },
  });
}
