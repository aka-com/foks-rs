import assert from 'node:assert/strict';
import test from 'node:test';
import {
  contentRevisions,
  freezeDto,
  readonlyMap,
  readonlySet,
} from '../src/chat/snapshots';
import type { ChatResult } from '../src/chat-contract';
type Inbox = Extract<ChatResult, { kind: 'inbox' }>;
function inbox(): Inbox {
  const channel = {
    id: 'ab'.repeat(16),
    name: 'design',
    description: null,
    admin: false,
    readable: true,
    writable: true,
    read_role: 'Member (0)',
    write_role: 'Member (0)',
  };
  return {
    kind: 'inbox',
    channels: [channel],
    conversations: [
      {
        channel,
        inbox_version: '1',
        read_through: '0',
        pending_read: null,
        unread: '9007199254740993',
        hidden: false,
        muted: false,
        preview: {
          sender: null,
          send_time: '1',
          insert_time: '1',
          content: { kind: 'text', text: 'hello' },
        },
      },
    ],
    blocked_channels: [],
    read_retry_pending: false,
    previews_incomplete: false,
    cursor: '1',
    head: '1',
    degraded: false,
  };
}
test('read, preference and enrichment status changes do not invalidate content', () => {
  const old = inbox();
  const next = structuredClone(old);
  next.conversations[0].read_through = '1';
  next.conversations[0].unread = '9007199254740992';
  next.conversations[0].muted = true;
  next.conversations[0].hidden = true;
  next.previews_incomplete = true;
  next.conversations[0].preview = null;
  const versions = new Map([[old.channels[0].id, 5]]);
  assert.deepEqual([...contentRevisions(old, next, versions)], [...versions]);
  next.conversations[0].unread = '9007199254740993';
  assert.equal(
    contentRevisions(old, next, versions).get(old.channels[0].id),
    6,
  );
});
test('authority, changed verified content and degraded fallback invalidate their channel', () => {
  const old = inbox();
  const next = structuredClone(old);
  const versions = new Map([[old.channels[0].id, 5]]);
  next.conversations[0].preview!.content = { kind: 'text', text: 'edited' };
  assert.equal(
    contentRevisions(old, next, versions).get(old.channels[0].id),
    6,
  );
  next.conversations[0].preview = old.conversations[0].preview;
  next.channels[0].readable = false;
  assert.equal(
    contentRevisions(old, next, versions).get(old.channels[0].id),
    6,
  );
  next.channels[0].readable = true;
  next.degraded = true;
  assert.equal(
    contentRevisions(old, next, versions).get(old.channels[0].id),
    6,
  );
});
test('published collection views and nested DTOs expose no mutation path', () => {
  const source = new Map([['channel', freezeDto(inbox())]]);
  const view = readonlyMap(source);
  source.clear();
  assert.equal(view.size, 1);
  assert.equal('set' in view, false);
  view.forEach((_value, _key, collection) => assert.equal(collection, view));
  assert.throws(() => {
    view.get('channel')!.channels[0].name = 'tampered';
  });
  const set = readonlySet(['channel']);
  assert.equal('add' in set, false);
  set.forEach((_value, _key, collection) => assert.equal(collection, set));
});
test('notification content does not advance for unchanged degraded projection', () => {
  const old = inbox(),
    next = structuredClone(old);
  next.degraded = true;
  const versions = new Map([[old.channels[0].id, 5]]);
  assert.equal(
    contentRevisions(old, next, versions, () => false).get(old.channels[0].id),
    5,
  );
  assert.equal(
    contentRevisions(old, next, versions).get(old.channels[0].id),
    6,
  );
});
