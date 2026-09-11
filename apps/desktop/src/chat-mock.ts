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
export function mockChat() {
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
  return async (storeId: string, action: ChatAction): Promise<ChatReply> => {
    let team = teams.get(storeId);
    if (!team) {
      const id = 'ab'.repeat(16);
      team = {
        channels: [
          {
            id,
            name: '',
            admin: false,
            readable: true,
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
      team.inboxVersion += 1n;
      team.versions.set(channel, team.inboxVersion);
      for (const wake of team.waiters) wake();
      team.waiters.clear();
    };
    let result: ChatResult;
    if (action.action === 'channels')
      result = {
        kind: 'channels',
        channels: [...team.channels],
        version: String(team.channels.length),
      };
    else if (action.action === 'history') {
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
        cursor: String(team.inboxVersion),
        head: String(team.inboxVersion),
        degraded: false,
        conversations: team.channels.map((channel) => {
          const rows = team.messages.get(channel.id) ?? [];
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
      const since = BigInt(action.since);
      if (team.inboxVersion <= since) {
        let wake!: () => void;
        const changed = new Promise<void>((resolve) => {
          wake = resolve;
          team.waiters.add(wake);
        });
        await Promise.race([
          changed,
          new Promise<void>((resolve) =>
            setTimeout(
              resolve,
              Math.min(action.timeout_milliseconds || 250, 250),
            ),
          ),
        ]);
        team.waiters.delete(wake);
      }
      result = {
        kind: 'poll',
        bumped: team.inboxVersion > since,
        inbox_version: String(team.inboxVersion),
      };
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
        create: action.action === 'prepare-channel',
        state: 'prepared',
        sequence: null,
        rejection_code: null,
      };
      team.operations.set(op.id, op);
      team.submissions.set(action.submission, { input, op });
      result = { kind: 'operation', operation: { ...op } };
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
          team.channels.push({
            id: op.channel,
            name: submitted.name,
            admin: submitted.admin,
            readable: true,
            read_role: submitted.admin ? 'Admin' : 'Member (0)',
            write_role: submitted.admin ? 'Admin' : 'Member (0)',
          });
          team.messages.set(op.channel, []);
          team.reads.set(op.channel, 0n);
          bump(op.channel);
        }
        if (submitted.action === 'prepare-message') {
          const rows = team.messages.get(op.channel) ?? [];
          op.sequence = String(rows.length + 1);
          rows.push({
            id: op.id,
            sequence: op.sequence,
            sender: '01' + 'ab'.repeat(32),
            content: { kind: 'text', text: submitted.text },
          });
          team.messages.set(op.channel, rows);
          team.reads.set(op.channel, BigInt(op.sequence));
          bump(op.channel);
        }
        op.state = 'confirmed';
      }
      result = { kind: 'operation', operation: { ...op } };
    }
    return {
      scope: {
        store: {
          profile: 'demo',
          account_alias: 'me',
          team_alias: storeId,
          team_id: '03' + 'ab'.repeat(32),
        },
        host: '02' + 'ab'.repeat(32),
        actor: '01' + 'ab'.repeat(32),
      },
      result,
    };
  };
}
