import type { AgentSnapshot } from '../model';
import { parseRole } from '../model/roles';
import type { ChatChannel, ChatMessage, ChatOperation } from '../chat-contract';

/**
 * The seeded states of the mock chat. `default` is one channel with one
 * message. `busy` is a team with several channels, a long unread run, grouped
 * authors and a sender absent from the roster. `failures` has ledger
 * operations that are unsent, unconfirmed or rejected on attempt, and a
 * channel whose saved message cannot be loaded. `integrity` has a history
 * page with missing predecessors, a degraded inbox, a stopped channel and a
 * muted and a hidden conversation.
 */
export const CHAT_MOCK_SCENARIOS = [
  'default',
  'busy',
  'failures',
  'integrity',
] as const;
export type ChatMockScenario = (typeof CHAT_MOCK_SCENARIOS)[number];

/** The scenario a mocked page's `chat-scenario` parameter names, else `default`. */
export function chatMockScenario(search: string): ChatMockScenario {
  const name = new URLSearchParams(search).get('chat-scenario');
  return CHAT_MOCK_SCENARIOS.find((s) => s === name) ?? 'default';
}

export const MOCK_CHAT_HOST = '02' + 'ab'.repeat(32);
export const MOCK_CHAT_ACTOR = '01' + 'ab'.repeat(32);
/** A UID in no fixture roster: a sender who has since left the team. */
export const MOCK_CHAT_DEPARTED = '01' + 'de'.repeat(32);
export const MOCK_CHAT_GENERAL = 'ab'.repeat(16);
export const MOCK_CHAT_DEPLOYS = 'b1'.repeat(16);
export const MOCK_CHAT_INCIDENTS = 'b2'.repeat(16);
export const MOCK_CHAT_RANDOM = 'b3'.repeat(16);
export const MOCK_CHAT_ARCHIVE = 'b4'.repeat(16);
/** The ledger operations of the `failures` scenario. */
export const MOCK_CHAT_PREPARED = 'f1'.repeat(16);
export const MOCK_CHAT_UNCERTAIN = 'f2'.repeat(16);
export const MOCK_CHAT_REJECTED = 'f3'.repeat(16);
/**
 * The message-order status a v0.1.9 host returns when it rejects a send; the
 * client records it, like six other statuses, as a definite rejection.
 */
export const MOCK_CHAT_REJECTION_CODE = 12006;

/** A scenario's initial state for one team. */
export interface ChatMockSeed {
  channels: ChatChannel[];
  /** Per channel, its messages in ascending sequence. */
  messages: Map<string, ChatMessage[]>;
  /** Per channel, the sequence read through. */
  reads: Map<string, bigint>;
  /** Ledger operations, each with its submission and prepared body. */
  operations: { operation: ChatOperation; submission: string; text: string }[];
  /** Operations whose attempt the host rejects, with the rejection code. */
  rejections: Map<string, number>;
  hidden: Set<string>;
  muted: Set<string>;
  /**
   * Channels the inbox reports stopped. The agent only echoes channels the
   * desktop asked it to block, so this is a mock shortcut for a channel the
   * desktop stopped earlier.
   */
  blocked: Set<string>;
  /** Per message ID, the predecessor sequences a page holding it reports. */
  missingPredecessors: Map<string, string[]>;
  degraded: boolean;
  /**
   * Channels whose saved-message load fails. The agent keeps every channel's
   * saved messages in one local store, so in production every channel fails
   * together; the mock confines the failure to the listed channels.
   */
  intentFailures: Set<string>;
}

const BASE_TIME = 1_700_000_000_000;

function channel(
  id: string,
  name: string,
  description: string | null,
  admin = false,
): ChatChannel {
  const role = admin ? 'Admin' : 'Member (0)';
  return {
    id,
    name,
    description,
    admin,
    readable: true,
    writable: true,
    read_role: role,
    write_role: role,
  };
}

function message(
  id: string,
  sequence: number,
  sender: string,
  time: number,
  text: string,
): ChatMessage {
  return {
    id,
    sequence: String(sequence),
    sender,
    send_time: String(time),
    insert_time: String(time + 1),
    content: { kind: 'text', text },
  };
}

/** A 32-hex message ID unique to a channel and sequence. */
function messageId(channel: string, sequence: number): string {
  return channel.slice(0, 2) + sequence.toString(16).padStart(30, '0');
}

/**
 * The team's other user members, Owner first, as the fixture roster lists
 * them. The local account's own party is left out: its messages are sent as
 * the chat actor.
 */
function roster(snapshot: AgentSnapshot | undefined, storeId: string) {
  const users = (snapshot?.parties ?? []).filter(
    (p) => p.store === storeId && p.party_kind === 'user' && p.label !== 'you',
  );
  const owner = (p: (typeof users)[number]) =>
    parseRole(p.source_role)?.kind === 'owner';
  return [...users.filter(owner), ...users.filter((p) => !owner(p))].map(
    (p) => p.party_id_hex,
  );
}

/** A run of messages from rotating senders, a few minutes apart. */
function conversation(
  id: string,
  count: number,
  senders: readonly string[],
  lines: readonly string[],
  start: number,
  first = 1,
): ChatMessage[] {
  const runs = [3, 1, 2, 1, 4, 2, 1];
  const rows: ChatMessage[] = [];
  let time = start;
  let run = 0;
  let left = runs[0];
  for (let i = 0; i < count; i++) {
    if (left === 0) {
      run += 1;
      left = runs[run % runs.length];
      time += 47 * 60_000;
    } else if (i) time += 45_000;
    left -= 1;
    const sequence = first + i;
    rows.push(
      message(
        messageId(id, sequence),
        sequence,
        senders[run % senders.length],
        time,
        lines[i % lines.length],
      ),
    );
  }
  return rows;
}

const GENERAL_LINES = [
  'Morning. The staging cluster is back after the node pool upgrade.',
  'Thanks. I will rerun the integration suite against it.',
  'The flaky login test passed three times in a row now.',
  'Reminder: the design review for the sharing dialog is at 14:00.',
  'Can someone review the key rotation change before lunch?',
  'Looking at it now.',
  'Left two comments, otherwise it looks fine.',
  'Release notes for 2.4 are drafted in the wiki.\nPlease add anything I missed.',
  'The new device enrollment flow is behind the flag on staging.',
  'I saw a timeout on the first sync after enrollment, filing an issue.',
  'Probably the cold cache. It should be fixed by the prefetch change.',
  'Agreed, closing it as a duplicate.',
  'Lunch order is going out in ten minutes.',
  'Out tomorrow afternoon, ping me before noon if you need anything.',
  'The backup job finished in 41 minutes, down from 58.',
  'Nice.',
];

const DEPLOY_LINES = [
  'Deploy 2.3.7 to staging started.',
  'Deploy 2.3.7 to staging finished. All health checks passed.',
  'Deploy 2.3.7 to production started.',
  'Deploy 2.3.7 to production finished. All health checks passed.',
  'Rollback window for 2.3.7 closes at 18:00.',
  'Deploy 2.4.0-rc1 to staging started.',
];

const INCIDENT_LINES = [
  'Elevated error rate on the sync endpoint since 09:12.',
  'Traced to one replica with a full disk. Draining it.',
  'Error rate is back to baseline.',
  'Postmortem draft is linked from the incident ticket.',
];

const RANDOM_LINES = [
  'Anyone have a recommendation for a mechanical keyboard?',
  'Anything with brown switches, the office is quiet.',
  'The coffee machine on the third floor is fixed.',
  'Book club picks the next title on Friday.',
  'Bike rack on the east side has space again.',
];

function seed(): ChatMockSeed {
  return {
    channels: [],
    messages: new Map(),
    reads: new Map(),
    operations: [],
    rejections: new Map(),
    hidden: new Set(),
    muted: new Set(),
    blocked: new Set(),
    missingPredecessors: new Map(),
    degraded: false,
    intentFailures: new Set(),
  };
}

/** The unnamed channel every team starts with. */
const generalChannel = () =>
  channel(MOCK_CHAT_GENERAL, '', 'A place for the whole team.');

/**
 * The general channel, holding one message, unread unless the local account
 * sent it: the agent advances the read pointer over the local account's sends.
 */
function general(state: ChatMockSeed, sender: string) {
  state.channels.push(generalChannel());
  state.messages.set(MOCK_CHAT_GENERAL, [
    message('cd'.repeat(16), 1, sender, BASE_TIME, 'Team chat is ready.'),
  ]);
  state.reads.set(MOCK_CHAT_GENERAL, sender === MOCK_CHAT_ACTOR ? 1n : 0n);
}

/** A seeded channel and its messages, read through `read`. */
function add(
  state: ChatMockSeed,
  row: ChatChannel,
  rows: ChatMessage[],
  read = rows.at(-1)?.sequence ?? '0',
) {
  state.channels.push(row);
  state.messages.set(row.id, rows);
  state.reads.set(row.id, BigInt(read));
}

function ledger(
  state: ChatMockSeed,
  id: string,
  operationState: 'prepared' | 'uncertain',
  text: string,
) {
  state.operations.push({
    operation: {
      id,
      channel: MOCK_CHAT_GENERAL,
      kind: 'send-message',
      state: operationState,
      confirmation: null,
      rejection_code: null,
    },
    submission: 'e' + id.slice(1),
    text,
  });
}

/** A scenario's initial state for the team `storeId` of `snapshot`. */
export function chatMockSeed(
  scenario: ChatMockScenario,
  snapshot: AgentSnapshot | undefined,
  storeId: string,
): ChatMockSeed {
  const members = roster(snapshot, storeId);
  // Every history row has a sender: the seed message has another member's,
  // the Owner's unless the local account owns the team, and the local
  // account's own when it is the only member.
  const owner = members[0] ?? MOCK_CHAT_ACTOR;
  // Others' messages, for rows past a channel's read pointer: the agent
  // advances that pointer over the local account's own sends.
  const others = members.length ? members : [MOCK_CHAT_DEPARTED];
  const state = seed();
  if (scenario === 'busy') {
    const people = [...members, MOCK_CHAT_ACTOR];
    state.channels.push(generalChannel());
    // The first 40 rows also rotate in a sender who has since left.
    const rows = conversation(
      MOCK_CHAT_GENERAL,
      40,
      [...people, MOCK_CHAT_DEPARTED],
      GENERAL_LINES,
      BASE_TIME,
    );
    rows.push(
      ...conversation(
        MOCK_CHAT_GENERAL,
        38,
        people,
        GENERAL_LINES.slice(5),
        Number(rows[39].send_time) + 47 * 60_000,
        41,
      ),
    );
    // The 42 unread rows are all from other members.
    rows.push(
      ...conversation(
        MOCK_CHAT_GENERAL,
        42,
        others,
        GENERAL_LINES.slice(3),
        Number(rows[77].send_time) + 47 * 60_000,
        79,
      ),
    );
    state.messages.set(MOCK_CHAT_GENERAL, rows);
    state.reads.set(MOCK_CHAT_GENERAL, 78n);
    add(
      state,
      channel(
        MOCK_CHAT_DEPLOYS,
        'deploys',
        'Deploy and rollback announcements.',
      ),
      // One poster: the roster's last member, which in Engineering's
      // fixture roster is its service account.
      conversation(
        MOCK_CHAT_DEPLOYS,
        6,
        others.slice(-1),
        DEPLOY_LINES,
        BASE_TIME + 3_600_000,
      ),
    );
    add(
      state,
      channel(
        MOCK_CHAT_INCIDENTS,
        'incidents',
        'Active incidents and follow-ups.',
        true,
      ),
      conversation(
        MOCK_CHAT_INCIDENTS,
        4,
        others,
        INCIDENT_LINES,
        BASE_TIME + 7_200_000,
      ),
      '2',
    );
    add(
      state,
      channel(MOCK_CHAT_RANDOM, 'random', null),
      conversation(MOCK_CHAT_RANDOM, 5, people, RANDOM_LINES, BASE_TIME),
    );
    return state;
  }
  general(state, owner);
  if (scenario === 'failures') {
    ledger(
      state,
      MOCK_CHAT_PREPARED,
      'prepared',
      'Rotating the staging certificates tonight.',
    );
    ledger(
      state,
      MOCK_CHAT_UNCERTAIN,
      'uncertain',
      'Can someone confirm the 2.4 tag?',
    );
    ledger(
      state,
      MOCK_CHAT_REJECTED,
      'prepared',
      'Incident summary for this morning.',
    );
    state.rejections.set(MOCK_CHAT_REJECTED, MOCK_CHAT_REJECTION_CODE);
    add(
      state,
      channel(
        MOCK_CHAT_DEPLOYS,
        'deploys',
        'Deploy and rollback announcements.',
      ),
      conversation(
        MOCK_CHAT_DEPLOYS,
        2,
        [owner],
        DEPLOY_LINES,
        BASE_TIME + 3_600_000,
      ),
    );
    state.intentFailures.add(MOCK_CHAT_DEPLOYS);
  } else if (scenario === 'integrity') {
    // Rows 1 and 2 are not held, and the page that starts at row 3 reports
    // them as predecessors it could not check.
    const deploys = conversation(
      MOCK_CHAT_DEPLOYS,
      6,
      [owner],
      DEPLOY_LINES,
      BASE_TIME + 3_600_000,
      3,
    );
    add(
      state,
      channel(
        MOCK_CHAT_DEPLOYS,
        'deploys',
        'Deploy and rollback announcements.',
      ),
      deploys,
    );
    state.missingPredecessors.set(deploys[0].id, ['1', '2']);
    add(
      state,
      channel(
        MOCK_CHAT_INCIDENTS,
        'incidents',
        'Active incidents and follow-ups.',
        true,
      ),
      conversation(
        MOCK_CHAT_INCIDENTS,
        3,
        [owner],
        INCIDENT_LINES,
        BASE_TIME + 7_200_000,
      ),
    );
    state.blocked.add(MOCK_CHAT_INCIDENTS);
    add(
      state,
      channel(MOCK_CHAT_RANDOM, 'random', null),
      conversation(MOCK_CHAT_RANDOM, 5, others, RANDOM_LINES, BASE_TIME),
      '2',
    );
    state.muted.add(MOCK_CHAT_RANDOM);
    add(
      state,
      channel(MOCK_CHAT_ARCHIVE, 'archive', 'Notes from the previous project.'),
      conversation(
        MOCK_CHAT_ARCHIVE,
        2,
        [owner],
        GENERAL_LINES,
        BASE_TIME - 86_400_000,
      ),
    );
    state.hidden.add(MOCK_CHAT_ARCHIVE);
    state.degraded = true;
  }
  return state;
}
