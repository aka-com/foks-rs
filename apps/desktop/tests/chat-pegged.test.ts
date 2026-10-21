import assert from 'node:assert/strict';
import test from 'node:test';
import {
  sameContent,
  type ChatContent,
  type ChatConversation,
} from '../src/chat-contract';
import {
  contentNotice,
  contentSummary,
  previewLine,
} from '../src/chat/presentation';

const target = 'ef'.repeat(16);
const pegged = (
  action: 'reply' | 'reaction' | 'edit' | 'delete',
): ChatContent => ({ kind: 'pegged', action, target });

test('pegged messages summarise as reply, reaction, edit and deletion placeholders', () => {
  assert.deepEqual(
    (['reply', 'reaction', 'edit', 'delete'] as const).map((action) =>
      contentSummary(pegged(action)),
    ),
    [
      'Replied to a message',
      'Reacted to a message',
      'Edited a message',
      'Deleted a message',
    ],
  );
  assert.equal(
    contentNotice(pegged('reaction')),
    'Reacted to a message. This version does not show the change.',
  );
  assert.equal(
    contentNotice({ kind: 'oversized' }),
    'This message exceeds the desktop display limit.',
  );
  assert.equal(
    contentNotice({ kind: 'unsupported' }),
    'This message type is not supported yet.',
  );
  assert.equal(contentSummary({ kind: 'text', text: 'hi' }), 'hi');
  assert.equal(contentSummary({ kind: 'unsupported' }), 'Unsupported message');
});

test('an inbox preview of a pegged message names the action', () => {
  const conversation = {
    preview: {
      sender: null,
      send_time: '1700000000000',
      insert_time: '1700000000001',
      content: pegged('edit'),
    },
  } as ChatConversation;
  assert.equal(previewLine(conversation), 'Team member: Edited a message');
});

test('pegged content compares action and target', () => {
  assert.ok(sameContent(pegged('reply'), pegged('reply')));
  assert.ok(!sameContent(pegged('reply'), pegged('edit')));
  assert.ok(
    !sameContent(pegged('reply'), {
      kind: 'pegged',
      action: 'reply',
      target: null,
    }),
  );
  assert.ok(!sameContent(pegged('reply'), { kind: 'unsupported' }));
  assert.ok(
    sameContent({ kind: 'text', text: 'a' }, { kind: 'text', text: 'a' }),
  );
  assert.ok(
    !sameContent({ kind: 'text', text: 'a' }, { kind: 'text', text: 'b' }),
  );
});
