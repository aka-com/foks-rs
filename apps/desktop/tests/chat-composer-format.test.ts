import assert from 'node:assert/strict';
import test from 'node:test';
import {
  formatEdit,
  mentionEdit,
  mentionToken,
  textBudget,
  type ComposerEdit,
  type ComposerFormat,
} from '../src/chat/composer-format';
import {
  channelRole,
  channelRoster,
  matchMembers,
  MENTION_CHOICES,
  type ChatMember,
} from '../src/chat/mentions';
import { CHAT_TEXT_BYTES } from '../src/chat-limits';

/** The draft after `edit`, with `[` and `]` around the selection. */
function applied(value: string, edit: ComposerEdit): string {
  const text = value.slice(0, edit.from) + edit.text + value.slice(edit.to);
  return (
    text.slice(0, edit.selectionStart) +
    '[' +
    text.slice(edit.selectionStart, edit.selectionEnd) +
    ']' +
    text.slice(edit.selectionEnd)
  );
}

/** Formats the `[`…`]` selection of `marked`. */
function format(kind: ComposerFormat, marked: string): string {
  const start = marked.indexOf('[');
  const end = marked.indexOf(']') - 1;
  const value = marked.replace('[', '').replace(']', '');
  return applied(value, formatEdit(kind, value, start, end));
}

test('inline formats wrap the selection and remove markers they find around it', () => {
  assert.equal(
    format('bold', 'is [below 0.05%] now'),
    'is **[below 0.05%]** now',
  );
  assert.equal(
    format('bold', 'is **[below 0.05%]** now'),
    'is [below 0.05%] now',
  );
  // A selection that takes in the markers is unwrapped too.
  assert.equal(format('bold', 'is [**below**] now'), 'is [below] now');
  assert.equal(format('italic', '[word]'), '*[word]*');
  assert.equal(format('italic', '*[word]*'), '[word]');
  assert.equal(format('code', 'run [make] now'), 'run `[make]` now');
  assert.equal(format('code', 'run `[make]` now'), 'run [make] now');
  // Italic inside bold markers adds its own rather than taking bold's.
  assert.equal(format('italic', '**[word]**'), '***[word]***');
  assert.equal(format('italic', '***[word]***'), '**[word]**');
  assert.equal(format('italic', '[**word**]'), '*[**word**]*');
  // Whitespace at the edges of the selection belongs to the text around it
  // and stays outside the markers.
  assert.equal(format('bold', 'a[ word ]b'), 'a **[word]** b');
  // No selection: an empty pair with the caret between the markers, which
  // the same key removes again.
  assert.equal(format('bold', 'say []'), 'say **[]**');
  assert.equal(format('bold', 'say **[]**'), 'say []');
  assert.equal(format('code', '[]'), '`[]`');
  assert.equal(format('code', '`[]`'), '[]');
  assert.equal(format('italic', '*[]*'), '[]');
  // A caret between bold markers takes an italic pair of its own.
  assert.equal(format('italic', '**[]**'), '***[]***');
});

test('a line that starts and ends with separate spans is not one span', () => {
  // `message-text.tsx` reads `**Name:**` and `**(req)**` as two spans, so
  // the key wraps the line rather than stripping its outer markers.
  assert.equal(
    format('bold', '[**Name:** a **(req)**]'),
    '**[**Name:** a **(req)**]**',
  );
  assert.equal(format('italic', '[*a* b *c*]'), '*[*a* b *c*]*');
  assert.equal(format('code', '[`a` b `c`]'), '`[`a` b `c`]`');
  // Nor is a selection between the inner markers of two spans enclosed.
  assert.equal(format('bold', '**[a** b **c]**'), '****[a** b **c]****');
  // Bold and italic still combine and come apart.
  assert.equal(format('italic', '[***word***]'), '[**word**]');
  assert.equal(format('bold', '**[*word*]**'), '[*word*]');
  // Across lines, such lines are wrapped, not stripped.
  const edit = formatEdit(
    'bold',
    '**Name:** a **(req)**\n**Kind:** b **(opt)**',
    0,
    43,
  );
  assert.equal(edit.removed, false);
  assert.equal(
    format('bold', '[**Name:** a **(req)**\n**Kind:** b **(opt)**]'),
    '[****Name:** a **(req)****\n****Kind:** b **(opt)****]',
  );
});

test('a caret between neighbouring spans is not an empty pair', () => {
  assert.equal(format('italic', '*a*[]*b*'), '*a**[]**b*');
  assert.equal(format('code', '`a`[]`b`'), '`a``[]``b`');
  assert.equal(format('bold', '**a**[]**b**'), '**a****[]****b**');
  // An empty pair typed right after a span is still removed.
  assert.equal(format('italic', '*a**[]*'), '*a*[]');
  assert.equal(format('bold', '**a****[]**'), '**a**[]');
});

test('inline formats wrap each non-blank line of a selection across lines', () => {
  assert.equal(
    format('bold', '[first\n\n  second]'),
    '[**first**\n\n  **second**]',
  );
  const edit = formatEdit('code', 'a\nb', 0, 3);
  assert.equal(edit.removed, false);
  // The same key again removes the markers from each line.
  assert.equal(format('bold', '[**alpha**\n**beta**]'), '[alpha\nbeta]');
  assert.equal(formatEdit('bold', '**a**\n**b**', 0, 11).removed, true);
  assert.equal(format('italic', '[*a*\n *b* ]'), '[a\n b ]');
  // Bold lines are not italic ones: italic adds its own markers.
  assert.equal(format('italic', '[**a**\n**b**]'), '[***a***\n***b***]');
  // With only some lines wrapped, the rest are wrapped to match.
  assert.equal(format('bold', '[**a**\nb]'), '[**a**\n**b**]');
});

test('a format across lines keeps list and quote markers first', () => {
  assert.equal(format('bold', '[- one\n- two]'), '[- **one**\n- **two**]');
  assert.equal(format('bold', '[- **one**\n- **two**]'), '[- one\n- two]');
  assert.equal(
    format('italic', '[* one\n2. two\n> three]'),
    '[* *one*\n2. *two*\n> *three*]',
  );
  // A selection that starts within a line takes no marker from it.
  assert.equal(format('bold', '- o[ne\n- two]'), '- o[**ne**\n- **two**]');
});

test('a format across lines leaves code fences and their lines as typed', () => {
  assert.equal(
    format('bold', '[say\n```\nmake\n```\ndone]'),
    '[**say**\n```\nmake\n```\n**done**]',
  );
  // A selection that starts inside a fence leaves the fence's remaining lines.
  assert.equal(
    format('bold', '```\nmake\n[run\n```\ndone]'),
    '```\nmake\n[run\n```\n**done**]',
  );
});

test('a code block fences the selected lines whole', () => {
  assert.equal(
    format('codeBlock', 'intro\nkub[ectl get] pods\ndone'),
    'intro\n```\n[kubectl get pods]\n```\ndone',
  );
  assert.equal(
    format('codeBlock', '[one\ntwo\n]three'),
    '```\n[one\ntwo]\n```\nthree',
  );
  assert.equal(format('codeBlock', '[]'), '```\n[]\n```');
});

test('the @name at the caret is read as message text reads a mention', () => {
  assert.deepEqual(mentionToken('cc @pri', 7, 7), {
    start: 3,
    end: 7,
    query: 'pri',
  });
  assert.deepEqual(mentionToken('@', 1, 1), { start: 0, end: 1, query: '' });
  assert.deepEqual(mentionToken('(@sam.o', 7, 7)?.query, 'sam.o');
  // Before a following word, inside an email address, inside a word, after
  // another @, or with text selected, nothing is completed.
  assert.equal(mentionToken('@pri x', 2, 2), null);
  assert.equal(mentionToken('bob@example', 11, 11), null);
  assert.equal(mentionToken('a@b', 3, 3), null);
  assert.equal(mentionToken('@@b', 3, 3), null);
  assert.equal(mentionToken('@pri', 1, 4), null);
  assert.equal(mentionToken('no mention', 10, 10), null);
  // A completion replaces the typed name and leaves a space after it.
  const token = mentionToken('cc @pri ok', 7, 7);
  assert.ok(token);
  assert.equal(
    applied('cc @pri ok', mentionEdit(token, 'priya.n')),
    'cc @priya.n [] ok',
  );
});

test('the space left is stated in bytes from 90% of the limit', () => {
  assert.equal(textBudget(0, CHAT_TEXT_BYTES), null);
  assert.equal(
    textBudget(Math.ceil(CHAT_TEXT_BYTES * 0.9) - 1, CHAT_TEXT_BYTES),
    null,
  );
  assert.deepEqual(textBudget(62_236, CHAT_TEXT_BYTES), {
    over: false,
    label: '3.3 KB left',
    detail:
      '3,300 of 65,536 bytes left. Accented letters, symbols and emoji take 2 to 4 bytes each.',
  });
  assert.equal(textBudget(65_000, CHAT_TEXT_BYTES)?.label, '536 bytes left');
  assert.equal(textBudget(65_535, CHAT_TEXT_BYTES)?.label, '1 byte left');
  assert.equal(textBudget(65_536, CHAT_TEXT_BYTES)?.label, '0 bytes left');
  const over = textBudget(65_800, CHAT_TEXT_BYTES);
  assert.equal(over?.over, true);
  assert.equal(over?.label, '264 bytes over the limit');
  assert.match(over?.detail ?? '', /^264 bytes over the 65,536-byte limit\./);
});

const roster: ChatMember[] = [
  { username: 'sam.ortiz', role: { role: 'Owner' } },
  { username: 'priya.n', role: { role: 'Admin' } },
  { username: 'dana.okafor', role: { role: 'Member', visibility: 0 } },
  { username: 'deploy-bot', role: { role: 'Member', visibility: 0 } },
  { username: 'lee', role: { role: 'Member', visibility: 2 } },
];

test('a channel offers the members its read role admits', () => {
  assert.deepEqual(channelRole('Member (2)'), {
    role: 'Member',
    visibility: 2,
  });
  assert.deepEqual(channelRole('Admin'), { role: 'Admin' });
  assert.equal(channelRole('Reader'), null);
  assert.deepEqual(channelRoster(roster, channelRole('Member (0)')), {
    members: roster,
    excluded: 0,
  });
  const admins = channelRoster(roster, channelRole('Admin'));
  assert.deepEqual(
    admins.members.map((m) => m.username),
    ['sam.ortiz', 'priya.n'],
  );
  assert.equal(admins.excluded, 3);
  // A visibility band admits members at or above it.
  assert.deepEqual(
    channelRoster(roster, channelRole('Member (1)')).members.map(
      (m) => m.username,
    ),
    ['sam.ortiz', 'priya.n', 'lee'],
  );
  // A read role this does not know filters no one; the server decides.
  assert.equal(channelRoster(roster, channelRole('Reader')).excluded, 0);
});

test('names match from their start or from a part after a separator', () => {
  const names = (query: string) =>
    matchMembers(roster, query).map((m) => m.username);
  assert.deepEqual(names(''), [
    'dana.okafor',
    'deploy-bot',
    'lee',
    'priya.n',
    'sam.ortiz',
  ]);
  assert.deepEqual(names('d'), ['dana.okafor', 'deploy-bot']);
  // The start of the name ranks above a later part.
  assert.deepEqual(
    matchMembers(
      [...roster, { username: 'bot.o', role: { role: 'Admin' } }],
      'bo',
    ).map((m) => m.username),
    ['bot.o', 'deploy-bot'],
  );
  assert.deepEqual(names('ortiz'), ['sam.ortiz']);
  // Compared as usernames are: `.`, `-` and `_` alike, case ignored.
  assert.deepEqual(names('Sam_O'), ['sam.ortiz']);
  assert.deepEqual(names('x'), []);
  const many = Array.from({ length: 20 }, (_, i) => ({
    username: `user${String(i).padStart(2, '0')}`,
    role: { role: 'Member', visibility: 0 },
  }));
  assert.equal(matchMembers(many, 'user').length, MENTION_CHOICES);
});
