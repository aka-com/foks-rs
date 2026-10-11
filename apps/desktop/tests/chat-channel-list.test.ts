/**
 * The Chat column's channel list as data: the order `listChannels` gives each
 * team's channels, what a row's preview line says, and the display
 * preferences the column stores.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import {
  listChannels,
  neverMessaged,
  previewLine,
} from '../src/chat/presentation';
import {
  rememberChannelPreviews,
  rememberChannelSort,
  rememberTeamFolded,
  storedChannelPreviews,
  storedChannelSort,
  storedFoldedTeams,
} from '../src/sidebar-prefs';
import type {
  ChatChannel,
  ChatConversation,
  ChatPreview,
} from '../src/chat-contract';
import { installDom } from './lib/dom-harness';

function channel(name: string, id = name || 'general'): ChatChannel {
  return {
    id,
    name,
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'Member (0)',
    write_role: 'Member (0)',
  };
}

function conversation(
  row: ChatChannel,
  options: {
    version?: string;
    at?: string;
    hidden?: boolean;
    unread?: string;
    read?: string;
    preview?: Partial<ChatPreview>;
  } = {},
): ChatConversation {
  return {
    channel: row,
    inbox_version: options.version ?? '1',
    read_through: options.read ?? '0',
    pending_read: null,
    unread: options.unread ?? '0',
    hidden: options.hidden ?? false,
    muted: false,
    preview:
      options.at === undefined && options.preview === undefined
        ? null
        : {
            sender: 'aa',
            send_time: options.at ?? '1',
            insert_time: options.at ?? '1',
            content: { kind: 'text', text: 'hello' },
            ...options.preview,
          },
  };
}

const names = (listed: ReturnType<typeof listChannels>) =>
  listed.map(({ channel: row }) => row.name || 'general');

test('channels list the general channel first and the rest A to Z, whatever order the agent sends', () => {
  const general = channel('');
  const zeta = channel('zeta');
  const alpha = channel('alpha');
  const quiet = channel('quiet');
  const stale = channel('aardvark');
  // The agent lists conversations by inbox version, newest first, and the
  // server issues a new version when the reader's read pointer rises. A
  // channel just read would therefore come first; the list does not follow.
  const conversations = [
    conversation(zeta, { version: '9' }),
    conversation(alpha, { version: '5' }),
    conversation(stale, { version: '7', hidden: true }),
    conversation(general, { version: '3' }),
  ];
  const all = [general, zeta, alpha, quiet, stale];
  assert.deepEqual(names(listChannels(all, conversations)), [
    'general',
    'alpha',
    'quiet',
    'zeta',
    'aardvark',
  ]);
  // Raising one conversation's version, which is what reading it does,
  // moves nothing.
  const read = [
    conversation(alpha, { version: '10' }),
    ...conversations.filter(({ channel: row }) => row !== alpha),
  ];
  assert.deepEqual(
    names(listChannels(all, read)),
    names(listChannels(all, conversations)),
  );
});

test('recent activity orders by the previewed message time and never by inbox version', () => {
  const general = channel('');
  const alpha = channel('alpha');
  const zeta = channel('zeta');
  const quiet = channel('quiet');
  const later = channel('later');
  const hidden = channel('archive');
  const conversations = [
    conversation(general, { version: '9', at: '1000' }),
    conversation(alpha, { version: '1', at: '3000' }),
    conversation(zeta, { version: '2', at: '2000' }),
    conversation(later, { version: '8' }),
    conversation(hidden, { version: '7', at: '9000', hidden: true }),
  ];
  const all = [general, alpha, zeta, quiet, later, hidden];
  // Channels with a preview come newest first; those with none follow by
  // name, and hidden conversations stay last in this order too.
  assert.deepEqual(names(listChannels(all, conversations, 'recent')), [
    'alpha',
    'zeta',
    'general',
    'later',
    'quiet',
    'archive',
  ]);
  // Equal times fall back to the alphabetical order.
  const tied = [
    conversation(zeta, { at: '5000' }),
    conversation(alpha, { at: '5000' }),
  ];
  assert.deepEqual(names(listChannels([zeta, alpha], tied, 'recent')), [
    'alpha',
    'zeta',
  ]);
});

test('a preview line names the sender, the reader as You, and withholds an unverifiable sender', () => {
  const row = channel('ops');
  const known = new Map([['aa', 'sam.ortiz']]);
  assert.equal(previewLine(undefined), '');
  assert.equal(previewLine(conversation(row)), '');
  assert.equal(
    previewLine(conversation(row, { at: '1' }), null, known),
    'sam.ortiz: hello',
  );
  assert.equal(
    previewLine(conversation(row, { at: '1' }), 'aa', known),
    'You: hello',
  );
  assert.equal(
    previewLine(conversation(row, { preview: { sender: null } })),
    'Team member: hello',
  );
  assert.equal(
    previewLine(
      conversation(row, { preview: { content: { kind: 'oversized' } } }),
      null,
      known,
    ),
    'sam.ortiz: Message too large to display',
  );
  assert.equal(
    previewLine(
      conversation(row, { preview: { content: { kind: 'unverifiable' } } }),
      null,
      known,
    ),
    'Unverifiable message',
  );
});

test('only a channel the inbox states is empty reads as never messaged', () => {
  const row = channel('ops');
  assert.equal(neverMessaged({ channel: row }), true);
  assert.equal(
    neverMessaged({ channel: row, conversation: conversation(row) }),
    true,
  );
  // A conversation with messages and no preview — withheld or unavailable —
  // is not empty.
  assert.equal(
    neverMessaged({
      channel: row,
      conversation: conversation(row, { read: '4' }),
    }),
    false,
  );
  assert.equal(
    neverMessaged({
      channel: row,
      conversation: conversation(row, { unread: '2' }),
    }),
    false,
  );
});

test('the column preferences default, persist, store no team id and tolerate unavailable storage', () => {
  const dom = installDom();
  assert.equal(storedChannelSort(), 'alphabetical');
  assert.equal(storedChannelPreviews(), true);
  assert.equal(storedFoldedTeams()('team:household'), false);
  rememberChannelSort('recent');
  rememberChannelPreviews(false);
  rememberTeamFolded('team:household', true);
  rememberTeamFolded('team:eng', true);
  rememberTeamFolded('team:eng', false);
  assert.equal(storedChannelSort(), 'recent');
  assert.equal(storedChannelPreviews(), false);
  const folded = storedFoldedTeams();
  assert.equal(folded('team:household'), true);
  assert.equal(folded('team:eng'), false);
  // The fold set holds hashes, not the references, which name local
  // profiles and aliases.
  const raw = window.localStorage.getItem('chatFoldedTeams') ?? '';
  assert.doesNotMatch(raw, /household/);
  assert.equal((JSON.parse(raw) as string[]).length, 1);
  // The set is bounded: the oldest entries go first.
  for (let index = 0; index < 300; index++)
    rememberTeamFolded(`team:${index}`, true);
  const bounded = JSON.parse(
    window.localStorage.getItem('chatFoldedTeams') ?? '[]',
  ) as string[];
  assert.equal(bounded.length, 256);
  assert.equal(storedFoldedTeams()('team:299'), true);
  assert.equal(storedFoldedTeams()('team:household'), false);
  // Unexpected values read as the defaults.
  window.localStorage.setItem('chatChannelSort', 'newest');
  window.localStorage.setItem('chatFoldedTeams', '{"not":"a list"}');
  assert.equal(storedChannelSort(), 'alphabetical');
  assert.equal(storedFoldedTeams()('team:299'), false);
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    get() {
      throw new Error('Storage blocked');
    },
  });
  assert.equal(storedChannelSort(), 'alphabetical');
  assert.equal(storedChannelPreviews(), true);
  assert.equal(storedFoldedTeams()('team:household'), false);
  assert.doesNotThrow(() => {
    rememberChannelSort('recent');
    rememberChannelPreviews(false);
    rememberTeamFolded('team:household', true);
  });
  dom.window.close();
});
