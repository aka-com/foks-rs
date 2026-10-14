import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import type { ChatMessage } from '../src/chat-contract';
import {
  findOffsets,
  findRanges,
  findSnippet,
  foldFindText,
  scanPage,
} from '../src/chat/find';

function message(
  sequence: number,
  content: ChatMessage['content'],
): ChatMessage {
  return {
    id: `m${sequence}`,
    sequence: String(sequence),
    sender: 'aa'.repeat(16),
    send_time: '1700000000000',
    insert_time: '1700000000001',
    content,
  };
}

test('folding keeps every offset of the text it folds', () => {
  for (const text of ['Canary', 'İstanbul canary', 'ẞ straße 𐐀𐐨', 'DÉJÀ vu']) {
    const folded = foldFindText(text);
    assert.equal(folded.length, text.length, text);
  }
  assert.equal(foldFindText('CANARY Release'), 'canary release');
  // "İ" lowercases to two scalars; the first is kept, as the agent keeps it,
  // so the text keeps its length.
  assert.equal(foldFindText('İx'), 'ix');
  assert.deepEqual(findOffsets('aaaa', 'aa'), [0, 2]);
  assert.deepEqual(findOffsets('canary', ''), []);
});

test('an older-page scan keeps a reference to each matching text row, newest first', () => {
  const page = [
    message(3, { kind: 'text', text: 'canary at 5%' }),
    message(4, { kind: 'unverifiable' }),
    message(5, { kind: 'text', text: 'rollback' }),
    message(6, { kind: 'text', text: 'Promote the CANARY' }),
  ];
  assert.deepEqual(
    scanPage(page, 'Canary').map((hit) => [hit.sequence, hit.text]),
    [
      ['6', 'Promote the CANARY'],
      ['3', 'canary at 5%'],
    ],
  );
  assert.deepEqual(scanPage(page, '   '), []);
});

test('a snippet puts the match on one line with its context', () => {
  assert.deepEqual(
    findSnippet('first line\n\nthen the Canary went out', 'canary', 9),
    {
      before: '…then the ',
      match: 'Canary',
      after: ' went out',
    },
  );
  assert.deepEqual(findSnippet('canary', 'CANARY'), {
    before: '',
    match: 'canary',
    after: '',
  });
  assert.equal(findSnippet('rollback', 'canary'), null);
});

test('matches span inline formatting but not paragraphs, controls or screen-reader text', () => {
  const { document } = new JSDOM(`<div id="list">
    <article data-message="a"><div class="chat-message-text">
      <p>The <strong>can</strong>ary is <span class="chat-mention">@sam</span>'s<span class="offscreen">, mentions canary</span></p>
      <p>canary</p>
      <div class="chat-code"><pre>canary</pre><button>canary</button></div>
    </div></article>
    <article data-operation="b"><div class="chat-message-text"><p>canary</p></div></article>
    <article data-message="c"><div class="chat-message-text"><p>can</p><p>ary</p></div></article>
  </div>`).window;
  const root = document.getElementById('list');
  assert.ok(root);
  const matches = findRanges(root, 'CANARY');
  assert.deepEqual(
    matches.map((match) => [match.row.dataset.message, match.range.toString()]),
    [
      ['a', 'canary'],
      ['a', 'canary'],
      ['a', 'canary'],
    ],
  );
  // The first match starts in the bold text and ends after it.
  assert.equal(
    matches[0].range.startContainer.parentElement?.tagName,
    'STRONG',
  );
  assert.equal(matches[0].range.endContainer.nodeValue, 'ary is ');
  assert.deepEqual(
    findRanges(root, "@sam's").map((match) => match.range.toString()),
    ["@sam's"],
  );
});
