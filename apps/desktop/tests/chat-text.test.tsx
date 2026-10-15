import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
import { JSDOM } from 'jsdom';
let vite: ViteDevServer;
let MessageText: typeof import('../src/chat/message-text').MessageText;
let safeChatLink: typeof import('../src/chat/message-text').safeChatLink;
let labelHosts: typeof import('../src/chat/message-text').labelHosts;
let chatLinkDisplay: typeof import('../src/chat/message-text').chatLinkDisplay;
let trimBareUrl: typeof import('../src/chat/message-text').trimBareUrl;
let chatMentions: typeof import('../src/chat/mentions').chatMentions;
let mentionKey: typeof import('../src/chat/mentions').mentionKey;
let relativeMessageTime: typeof import('../src/chat/presentation').relativeMessageTime;
let messageClock: typeof import('../src/chat/presentation').messageClock;
let quoteReply: typeof import('../src/chat/message-actions').quoteReply;
let quoteInsertion: typeof import('../src/chat/message-actions').quoteInsertion;
let ToastProvider: typeof import('../kit/toasts').ToastProvider;
let ToastController: typeof import('../kit/toasts').ToastController;
let MessageFindContext: typeof import('../src/chat/message-text').MessageFindContext;
let find: typeof import('../src/chat/find');
test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({
    MessageText,
    MessageFindContext,
    safeChatLink,
    labelHosts,
    chatLinkDisplay,
    trimBareUrl,
  } = await vite.ssrLoadModule('/src/chat/message-text.tsx'));
  find = (await vite.ssrLoadModule(
    '/src/chat/find.ts',
  )) as typeof import('../src/chat/find');
  ({ chatMentions, mentionKey } = await vite.ssrLoadModule(
    '/src/chat/mentions.ts',
  ));
  ({ relativeMessageTime, messageClock } = await vite.ssrLoadModule(
    '/src/chat/presentation.ts',
  ));
  ({ quoteReply, quoteInsertion } = await vite.ssrLoadModule(
    '/src/chat/message-actions.tsx',
  ));
  ({ ToastProvider, ToastController } =
    await vite.ssrLoadModule('/kit/toasts.tsx'));
});
test.after(async () => {
  await vite.close();
});
const actions = {
  copyText: async () => ({ ok: true as const }),
  openChatLink: async () => ({ ok: true as const }),
};
/**
 * The message's own markup. The toast provider draws its persistent live
 * regions after its children, and those are not the message's.
 */
const messageMarkup = (
  text: string,
  mentions?: ReturnType<typeof chatMentions>,
): string => {
  const html = renderToStaticMarkup(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(MessageText, { text, actions, mentions }),
    }),
  );
  const toasts = html.indexOf('<div id="toasts"');
  return toasts < 0 ? html : html.slice(0, toasts);
};
test('Basic source renders safe bounded Markdown without HTML or image fetching', () => {
  const text =
    '**bold** *em* `code`\n- first\n- second\n> quote\n```\n<x>\n```\n<img src="https://evil"> ![image](https://example.com/a) [bad](javascript:alert)';
  const html = messageMarkup(text);
  assert.match(html, /<strong>bold<\/strong>/);
  assert.match(html, /<ul>/);
  assert.match(html, /<blockquote>/);
  assert.match(html, /&lt;x&gt;/);
  assert.doesNotMatch(html, /<img|href="javascript/);
  assert.match(html, /Copy code/);
  assert.doesNotMatch(html, /Copy message/);
});
test('message timestamps are relative to the current time', () => {
  const now = Date.UTC(2026, 8, 18, 12);
  assert.equal(relativeMessageTime(String(now), now), 'just now');
  assert.equal(relativeMessageTime(String(now - 120_000), now), '2 min ago');
  assert.equal(relativeMessageTime(String(now - 60_000), now), '1 min ago');
  assert.equal(
    relativeMessageTime(String(now - 7_200_000), now),
    '2 hours ago',
  );
  assert.equal(relativeMessageTime(String(now - 3_600_000), now), '1 hour ago');
  assert.equal(relativeMessageTime(String(now - 86_400_000), now), '1 day ago');
  assert.equal(
    relativeMessageTime(String(now - 259_200_000), now),
    '3 days ago',
  );
  // Only a clock disagreement puts a message ahead of now; it is not an age.
  assert.equal(relativeMessageTime(String(now + 300_000), now), 'in 5 min');
});

test('unsafe and disguised links remain inert', () => {
  for (const value of [
    'javascript:alert(1)',
    'java\nscript:alert(1)',
    'javascript%3Aalert',
    'data:text/html,x',
    'file:///tmp/x',
    ' https://a',
    'https://a\\b',
    'https://user@a',
  ])
    assert.equal(safeChatLink(value), null);
  assert.equal(safeChatLink('https://example.com/a'), 'https://example.com/a');
});
test('a link label naming another host is marked and its destination host shown', () => {
  const html = messageMarkup(
    'Sign in at [https://bank.example/login](https://evil.example/)',
  );
  assert.match(
    html,
    /<a href="https:\/\/evil\.example\/" title="https:\/\/evil\.example\/" class="chat-link-mismatch"/,
  );
  assert.match(html, /Link text names a different host: /);
  assert.match(
    html,
    /<span class="offscreen">, opens evil\.example<\/span><\/a><bdi dir="ltr" aria-hidden="true" class="chat-link-host mismatch">\(evil\.example\)<\/bdi>/,
  );
  assert.deepEqual(
    chatLinkDisplay('https://bank.example/login', 'https://evil.example/'),
    {
      label: 'https://bank.example/login',
      host: 'evil.example',
      showHost: true,
      mismatchedHost: 'bank.example',
    },
  );
});

test('a link label that is the destination URL carries no host suffix or warning', () => {
  for (const text of [
    '[https://example.com/a](https://example.com/a)',
    '[https://example.com](https://example.com)',
  ]) {
    const html = messageMarkup(text);
    assert.match(
      html,
      /<a href="https:\/\/example\.com\/a?" title="https:\/\/example\.com\/a?" rel=/,
    );
    assert.doesNotMatch(html, /chat-link-host|chat-link-mismatch/);
  }
});

test('a label that is the destination URL but names another host carries the host suffix', () => {
  const url = 'https://evil.example/#@bank.example';
  assert.deepEqual(chatLinkDisplay(url, url), {
    label: url,
    host: 'evil.example',
    showHost: true,
    mismatchedHost: 'bank.example',
  });
  const html = messageMarkup(`[${url}](${url})`);
  assert.match(
    html,
    /<span class="offscreen">, opens evil\.example<\/span><\/a><bdi dir="ltr" aria-hidden="true" class="chat-link-host mismatch">\(evil\.example\)<\/bdi>/,
  );
});

test('a label that differs from the URL only in the case of ASCII letters carries no host suffix', () => {
  for (const label of ['HTTPS://BANK.EXAMPLE', 'https://Bank.Example/'])
    assert.equal(
      chatLinkDisplay(label, 'https://bank.example/').showHost,
      false,
      label,
    );
  // A non-ASCII letter the URL parser maps to an ASCII one is not ignored.
  assert.equal(
    chatLinkDisplay('https://\u212aey.example/', 'https://key.example/')
      .showHost,
    true,
  );
});

test('a plain-word label shows the destination host without a warning', () => {
  const html = messageMarkup('Notes are [here](https://docs.example/notes).');
  assert.match(html, /title="https:\/\/docs\.example\/notes"/);
  assert.match(
    html,
    />here<\/bdi><span class="offscreen">, opens docs\.example<\/span><\/a><bdi dir="ltr" aria-hidden="true" class="chat-link-host">\(docs\.example\)<\/bdi>/,
  );
  assert.doesNotMatch(html, /chat-link-mismatch|different host/);
});

test('an internationalized destination host is shown in its xn-- form', () => {
  // U+0430 CYRILLIC SMALL LETTER A in place of the Latin a.
  const html = messageMarkup('[here](https://\u0430pple.example/)');
  assert.match(html, /href="https:\/\/xn--pple-43d\.example\/"/);
  assert.match(html, /\(xn--pple-43d\.example\)/);
  assert.doesNotMatch(html, /chat-link-mismatch/);
  // A Unicode label for an ASCII host, and the reverse, are mismatches.
  assert.equal(
    chatLinkDisplay('b\u0430nk.example', 'https://bank.example/')
      .mismatchedHost,
    'xn--bnk-6cd.example',
  );
  assert.equal(
    chatLinkDisplay('bank.example', 'https://xn--bnk-6cd.example/')
      .mismatchedHost,
    'bank.example',
  );
  // The same IDN written in Unicode in both places is not.
  assert.equal(
    chatLinkDisplay(
      'https://b\u0430nk.example',
      safeChatLink('https://b\u0430nk.example')!,
    ).mismatchedHost,
    null,
  );
});

test('label hosts are read from words that read as URLs or host names', () => {
  const cases: [string, string[]][] = [
    ['https://Bank.Example/login', ['bank.example']],
    ['ftp://files.example', ['files.example']],
    ['bank.example', ['bank.example']],
    [' www.bank.example/login?x=1 ', ['www.bank.example']],
    ['bank.example.', ['bank.example']],
    ['bank.example:8443/x', ['bank.example']],
    ['192.0.2.7', ['192.0.2.7']],
    ['bank\u3002example', ['bank.example']],
    // Quotes, brackets, punctuation and surrounding words.
    ['<https://bank.example>', ['bank.example']],
    ['"bank.example"', ['bank.example']],
    ['(bank.example)', ['bank.example']],
    ['`bank.example`', ['bank.example']],
    ['bank.example,', ['bank.example']],
    ['Go to bank.example', ['bank.example']],
    ['https://bank.example (official)', ['bank.example']],
    ['bank.example or other.example', ['bank.example', 'other.example']],
    // Invisible characters do not hide a host name: format characters, and
    // default ignorable marks such as U+034F COMBINING GRAPHEME JOINER and the
    // variation selectors.
    ['bank\u200b.example', ['bank.example']],
    ['\u202ebank.example/login', ['bank.example']],
    ['bank.\u034fexample/login', ['bank.example']],
    ['bank\u034f.example', ['bank.example']],
    ['bank.\ufe0fexample', ['bank.example']],
    ['Sign in at bank.\ufe00example', ['bank.example']],
    ['bank.\u{e0100}example', ['bank.example']],
    ['bank.\u180bexample', ['bank.example']],
    ['https://bank.\u034fexample/login', ['bank.example']],
    // A combining mark neither ends a host name nor hides one.
    ['bank.ex\u0301ample', ['bank.xn--example-uge']],
    ['bank.e\u0301xample', ['bank.xn--xample-9ua']],
    // The URL parser rejects a label that starts with a mark, so that name
    // is kept as written and matches no destination.
    ['bank.\u0301example', ['bank.\u0301example']],
    // Other forms of the full stop separate labels; other forms of `/`, `?`
    // and `#` border a name but do not end the authority.
    ['bank\u2024example', ['bank.example']],
    ['bank\ufe52example', ['bank.example']],
    ['bank\uff0eexample', ['bank.example']],
    ['Login\uff0fbank.example', ['bank.example']],
    ['x\uff1fbank.example', ['bank.example']],
    ['x\uff03bank.example', ['bank.example']],
    // A Hangul filler is drawn as a blank, so it is not removed.
    ['secure-login\u3164paypal.com', ['secure-login\u3164paypal.com']],
    [
      '\uff42\uff41\uff4e\uff4b\uff0e\uff45\uff58\uff41\uff4d\uff50\uff4c\uff45',
      ['bank.example'],
    ],
    // A dotted name on either side of an @ is read; a plain name before it is
    // a user name.
    ['https://bank.example@evil.example/', ['bank.example', 'evil.example']],
    ['bank.example@evil.example', ['bank.example', 'evil.example']],
    ['x.y@z@bank.example', ['bank.example']],
    ['https://user@bank.example', ['bank.example']],
    ['alice@bank.example', ['bank.example']],
    // A file or product name reads as a host only with a scheme, path, port
    // or www. prefix.
    ['index.html', []],
    ['Node.js', []],
    ['config.yaml', []],
    ['https://index.html', ['index.html']],
    ['index.html/x', ['index.html']],
    ['www.index.html', ['www.index.html']],
    // An extension that is also a top-level domain makes a file name only
    // after a stem with an upper-case letter or an inner underscore, compared
    // without regard to case.
    ['README.md', []],
    ['README.MD', []],
    ['"README.md"', []],
    ['setup_env.sh', []],
    ['__init__.py', []],
    ['Main.java', []],
    ['docs.rs', ['docs.rs']],
    ['docs.RS', ['docs.rs']],
    ['crates.rs', ['crates.rs']],
    ['get.sh', ['get.sh']],
    ['(docs.rs)', ['docs.rs']],
    ['_docs.rs_', ['docs.rs']],
    ['README.md/x', ['readme.md']],
    ['https://README.md', ['readme.md']],
    ['www.PayPal.zip', ['www.paypal.zip']],
    ['security.center@bank.example', ['security.center', 'bank.example']],
    // Non-ASCII punctuation and suffixes around a host.
    ["bank.example's portal", ['bank.example']],
    ['bank.example\u2019s', ['bank.example']],
    ['\u201cbank.example\u201d', ['bank.example']],
    ['\u00abbank.example\u00bb', ['bank.example']],
    ['\u300cbank.example\u300d', ['bank.example']],
    ['{bank.example}', ['bank.example']],
    ['*bank.example*', ['bank.example']],
    ['_bank.example_', ['bank.example']],
    ['bank.example\u2014now', ['bank.example']],
    ['bank.example\u2026', ['bank.example']],
    // Schemes browsers accept with one slash or with backslashes.
    ['https:/bank.example', ['bank.example']],
    ['https:\\\\bank.example', ['bank.example']],
    ['https://intranet', ['intranet']],
    ['https://127.1', ['127.0.0.1']],
    ['https://a.b', ['a.b']],
    ['https://\u0301', []],
    ['HTTPS:\\bank.example', ['bank.example']],
    // A name after `?@` or `#@` at the end of the authority reads as the host
    // to a reader who takes the host to follow the last `@`.
    ['https://bank.example?@evil.example', ['bank.example', 'evil.example']],
    ['https://bank.example/#@evil.example', ['bank.example', 'evil.example']],
    ['https://bank.example\\?@evil.example', ['bank.example', 'evil.example']],
    [
      'https://bank.example#@https://evil.example',
      ['bank.example', 'evil.example'],
    ],
    ['bank.example#@evil.example', ['bank.example', 'evil.example']],
    ['bank.example?@evil.example', ['bank.example', 'evil.example']],
    ['bank.example/#@evil.example', ['bank.example', 'evil.example']],
    ['bank.example#@https://evil.example', ['bank.example', 'evil.example']],
    ['bank.example/?to=a@evil.example', ['bank.example']],
    ['x#@evil.example', []],
    ['https://bank.example/?next=@evil.example', ['bank.example']],
    ['https://bank.example/?to=a@evil.example', ['bank.example']],
    ['https://mastodon.social/@user', ['mastodon.social']],
    // A host before a scheme is read first.
    ['paypal.com/x:/evil.example', ['paypal.com', 'evil.example']],
    ['paypal.com:/evil.example', ['paypal.com', 'evil.example']],
    ['paypal.com.js:/evil.example', ['paypal.com.js', 'evil.example']],
    // A keycap borders a name; an enclosing mark inside a name does not.
    ['1\ufe0f\u20e3bank.example', ['bank.example']],
    ['1\u20e3bank.example', ['bank.example']],
    ['bank.e\u20e3xample', ['bank.xn--example-ef8c']],
    // A name longer than the longest accepted link is reported shortened.
    [`${'a'.repeat(300)}.example`, [`${'a'.repeat(300)}.example`]],
    [`${'a'.repeat(2048)}.example`, [`${'a'.repeat(253)}\u2026`]],
    // A host after many words is still read.
    [`${'word '.repeat(100)}bank.example`, ['bank.example']],
    ['here', []],
    ['the docs', []],
    ['1.5', []],
    ['v1.2.3', []],
    ['e.g', []],
    ['i.e.', []],
    ['', []],
  ];
  for (const [label, hosts] of cases)
    assert.deepEqual(labelHosts(label), hosts, label);
});

test('a label host is compared with the destination up to one leading www.', () => {
  // A host written with a Hangul filler is not the host without it.
  assert.equal(
    chatLinkDisplay(
      'secure-login\u3164paypal.com',
      'https://secure-loginpaypal.com/',
    ).mismatchedHost,
    'secure-login\u3164paypal.com',
  );
  for (const [label, href] of [
    ['https://127.1', 'https://127.0.0.1/'],
    ['https://a.b', 'https://a.b/'],
  ])
    assert.equal(chatLinkDisplay(label, href).mismatchedHost, null, label);
  // Same host, different path: the suffix is shown but nothing is flagged.
  assert.deepEqual(
    chatLinkDisplay('https://bank.example/login', 'https://bank.example/home'),
    {
      label: 'https://bank.example/login',
      host: 'bank.example',
      showHost: true,
      mismatchedHost: null,
    },
  );
  for (const [label, href] of [
    ['www.bank.example', 'https://bank.example/'],
    ['bank.example', 'https://www.bank.example/'],
  ])
    assert.equal(chatLinkDisplay(label, href).mismatchedHost, null, label);
  assert.equal(
    chatLinkDisplay('www.bank.example', 'https://login.bank.example/')
      .mismatchedHost,
    'www.bank.example',
  );
  // Any word naming another host is enough.
  assert.equal(
    chatLinkDisplay('bank.example, not evil.example', 'https://evil.example/')
      .mismatchedHost,
    'bank.example',
  );
  assert.equal(
    chatLinkDisplay('index.html', 'https://github.com/x/y').mismatchedHost,
    null,
  );
  const againstEvil: [string, string | null][] = [
    ['README.md', null],
    ['setup_env.sh', null],
    ['docs.rs', 'docs.rs'],
    ['get.sh', 'get.sh'],
    ['https://evil.example#@bank.example', 'bank.example'],
  ];
  for (const [label, named] of againstEvil)
    assert.equal(
      chatLinkDisplay(label, 'https://evil.example/').mismatchedHost,
      named,
      label,
    );
  assert.equal(
    chatLinkDisplay('security.center@bank.example', 'https://security.center/')
      .mismatchedHost,
    'bank.example',
  );
});

test('a link label is isolated from the bidirectional context of the text after it', () => {
  const html = messageMarkup(
    '[\u05e9\u05dc\u05d5\u05dd](https://evil.example/) after',
  );
  assert.match(
    html,
    /<bdi>\u05e9\u05dc\u05d5\u05dd<\/bdi><span class="offscreen">, opens evil\.example<\/span><\/a><bdi dir="ltr" aria-hidden="true" class="chat-link-host">\(evil\.example\)<\/bdi>/,
  );
});

test('a link label is drawn without the bidirectional controls it was compared without', () => {
  // U+202E would draw `bank.ac` as `bank.ca`, and `elpmaxe.knab` as
  // `bank.example`; drawn without it, the label reads as compared.
  assert.deepEqual(chatLinkDisplay('bank.\u202eac', 'https://bank.ac/'), {
    label: 'bank.ac',
    host: 'bank.ac',
    showHost: true,
    mismatchedHost: null,
  });
  assert.deepEqual(
    chatLinkDisplay('https://\u202eelpmaxe.knab', 'https://elpmaxe.knab/'),
    {
      label: 'https://elpmaxe.knab',
      host: 'elpmaxe.knab',
      showHost: false,
      mismatchedHost: null,
    },
  );
  assert.equal(
    chatLinkDisplay(
      '\u2067x\u200e\u200f\u061c\u202a\u2069',
      'https://a.example/',
    ).label,
    'x',
  );
  const html = messageMarkup('[bank.\u202eac](https://bank.ac/) after');
  assert.match(html, /<bdi>bank\.ac<\/bdi>/);
  assert.doesNotMatch(html, /\u202e/);
  // Other invisible characters do not reorder text and are drawn as sent.
  assert.equal(
    chatLinkDisplay('bank\u200b.example', 'https://bank.example/').label,
    'bank\u200b.example',
  );
});

test('reading the hosts of a long label takes linear time', () => {
  // Labels of one character, with no final label to end a match, and a word
  // without a scheme separator, were each quadratic for a pattern retried at
  // every position. 64K characters is the longest text MessageText formats.
  for (const label of [
    'a.'.repeat(32000),
    'a'.repeat(64000),
    `${'a'.repeat(64000)}.bb`,
    `https://${'a.'.repeat(32000)}`,
    `${'a:'.repeat(32000)}//bank.example`,
    `a.bb${'\u0301'.repeat(32000)}${'\u0316'.repeat(32000)}`,
    // Many file names with top-level-domain extensions in one stem run, each
    // of whose stems is tested for a mark.
    'a_a.md_'.repeat(9000),
    `x_${'a.md.'.repeat(12800)}`,
    `https://bank.example#@${'a.'.repeat(32000)}bb`,
    `bank.example#@${'a.'.repeat(32000)}bb`,
    `https://bank.example#@${'a'.repeat(64000)}`,
  ]) {
    const started = performance.now();
    labelHosts(label);
    const elapsed = performance.now() - started;
    assert.ok(elapsed < 1000, `${label.slice(0, 8)}… took ${elapsed} ms`);
  }
  const started = performance.now();
  messageMarkup(`[${'a.'.repeat(32000)}](https://evil.example/)`);
  assert.ok(performance.now() - started < 1000);
});

test('a bare URL is a link without the punctuation that ends its sentence', () => {
  const html = messageMarkup(
    'Notes: https://example.com/notes. See (https://example.com/a_(b)) and https://example.com/x?y=1),',
  );
  assert.match(
    html,
    /Notes: <a href="https:\/\/example\.com\/notes" title="https:\/\/example\.com\/notes" rel="noreferrer noopener"><bdi>https:\/\/example\.com\/notes<\/bdi><\/a>\. See \(<a href="https:\/\/example\.com\/a_\(b\)"/,
  );
  assert.match(
    html,
    /<bdi>https:\/\/example\.com\/a_\(b\)<\/bdi><\/a>\) and <a href="https:\/\/example\.com\/x\?y=1"/,
  );
  assert.match(html, /<bdi>https:\/\/example\.com\/x\?y=1<\/bdi><\/a>\),<\/p>/);
  assert.doesNotMatch(html, /chat-link-host|chat-link-mismatch/);
  assert.equal(
    trimBareUrl('https://example.com/a]"!?'),
    'https://example.com/a',
  );
  assert.equal(
    trimBareUrl('https://example.com/[a]'),
    'https://example.com/[a]',
  );
  // CJK sentence punctuation and unmatched full-width closers end the URL.
  assert.equal(
    trimBareUrl('https://example.com/path」。'),
    'https://example.com/path',
  );
  assert.equal(
    trimBareUrl('https://example.com/「a」'),
    'https://example.com/「a」',
  );
  // A trailing `_` is trimmed only when a `_` precedes the URL.
  assert.equal(trimBareUrl('https://example.com/a_'), 'https://example.com/a_');
  assert.equal(
    trimBareUrl('https://example.com_.', true),
    'https://example.com',
  );
  const emphasized = messageMarkup(
    'x _https://example.com_ y https://example.com/path。',
  );
  assert.match(
    emphasized,
    /x _<a href="https:\/\/example\.com\/" title="https:\/\/example\.com\/" rel="noreferrer noopener"><bdi>https:\/\/example\.com<\/bdi><\/a>_ y /,
  );
  assert.match(
    emphasized,
    /<a href="https:\/\/example\.com\/path"[^>]*><bdi>https:\/\/example\.com\/path<\/bdi><\/a>。<\/p>/,
  );
  assert.doesNotMatch(emphasized, /chat-link-mismatch/);
});

test('a bare URL passes the same checks as a link destination', () => {
  // safeChatLink rejects credentials, so the text stays inert.
  const html = messageMarkup(
    'https://user@evil.example/ x https://ex\u0430mple.com/ y https://evil.example/#@bank.example foohttps://example.com',
  );
  assert.doesNotMatch(html, /href="https:\/\/user@/);
  assert.match(
    html,
    /^<div class="chat-message-text"><p>https:\/\/user@evil\.example\/ x <a/,
  );
  // An internationalized host is followed by the host it resolves to.
  assert.match(
    html,
    /<bdi dir="ltr" aria-hidden="true" class="chat-link-host">\(xn--exmple-4nf\.com\)<\/bdi>/,
  );
  // A URL whose text names another host is marked like a disguised label.
  assert.match(
    html,
    /<a href="https:\/\/evil\.example\/#@bank\.example" title="[^"]*" class="chat-link-mismatch"/,
  );
  // A scheme that continues a word does not start a link.
  assert.match(html, /foohttps:\/\/example\.com<\/p>/);
});

test('bare-URL and mention scanning is linear in the text length', () => {
  for (const text of [
    `https://example.com/${')'.repeat(64000)}`,
    `https://example.com/${'.'.repeat(64000)}`,
    `@a${'.'.repeat(64000)}`,
    `@a${'.'.repeat(64000)}b`,
    `@a${'-'.repeat(64000)}b`,
    `@a${'.-'.repeat(32000)}b`,
    `https://example.com/${'。'.repeat(64000)}`,
    `_https://example.com/${'_'.repeat(64000)}`,
    '@a '.repeat(21000),
  ]) {
    const started = performance.now();
    messageMarkup(text, chatMentions(['a.b'], 'a.b'));
    const elapsed = performance.now() - started;
    assert.ok(elapsed < 1000, `${text.slice(0, 8)}… took ${elapsed} ms`);
  }
});

test('blank lines separate paragraphs; single line breaks do not', () => {
  assert.match(
    messageMarkup('one\ntwo'),
    /^<div class="chat-message-text"><p>one<\/p><p>two<\/p><\/div>/,
  );
  const html = messageMarkup(
    '\n\none\n\n\ntwo\n \n- a\n- b\n\t\n> q\r\n\r\n```\r\nx\r\n```\r\n',
  );
  // Leading blank lines draw nothing; a run of blank lines is one gap. A
  // line of only whitespace is blank, and CRLF line ends read as LF.
  assert.ok(
    html.startsWith(
      '<div class="chat-message-text"><p>one</p><p class="chat-gap">two</p>' +
        '<ul class="chat-gap"><li>a</li><li>b</li></ul>' +
        '<blockquote class="chat-gap">q</blockquote>' +
        '<div class="chat-code chat-gap"><pre',
    ),
    html,
  );
});

test('consecutive quote lines are one quote', () => {
  assert.match(
    messageMarkup('> **@lee** · 9:14\n> first *line*\n> second\nreply'),
    /^<div class="chat-message-text"><blockquote><strong>@lee<\/strong> · 9:14<br\/>first <em>line<\/em><br\/>second<\/blockquote><p>reply<\/p><\/div>/,
  );
  // A blank line between them starts a second quote.
  assert.equal(
    (messageMarkup('> one\n\n> two').match(/<blockquote/g) ?? []).length,
    2,
  );
});

test('a quote reply is Basic text naming the author and the first lines', () => {
  const at = '1700000000001';
  const clock = messageClock(at);
  const mentions = chatMentions(['priya.n'], 'satoshi');
  assert.equal(
    quoteReply('priya.n', at, 'Deploy is done.\nChecks are green.', mentions),
    `> **@priya.n** · ${clock}\n> Deploy is done.\n> Checks are green.\n\n`,
  );
  // A name off the roster, such as a shortened party ID, is not written as
  // an @name.
  assert.equal(
    quoteReply('01cdcdcd', at, 'hi', mentions),
    `> **01cdcdcd** · ${clock}\n> hi\n\n`,
  );
  // Fence lines, blank lines and one level of quoting are left out; a third
  // line is cut and the quote ends in an ellipsis.
  assert.equal(
    quoteReply('lee', at, '> earlier\n\n```\ncode\n```\nthird'),
    `> **lee** · ${clock}\n> earlier\n> code…\n\n`,
  );
  // A long line is cut at a word boundary, counted in characters.
  const long = `${'word '.repeat(30)}end`;
  const quoted = quoteReply('lee', at, long).split('\n')[1];
  assert.ok(quoted.endsWith('word…'), quoted);
  assert.ok(Array.from(quoted).length <= 102, quoted);
  const emoji = '😀'.repeat(150);
  const cut = quoteReply('lee', at, emoji).split('\n')[1];
  assert.equal(cut, `> ${'😀'.repeat(99)}…`);
  // The ellipsis marks the line that was cut, not a complete line after it.
  assert.equal(
    quoteReply('lee', at, `${long}\nShort second line.`)
      .split('\n')
      .slice(1, 3)
      .join('\n'),
    `${quoted}\n> Short second line.`,
  );
  // The quote starts on a line of its own.
  assert.equal(quoteInsertion('', '> q\n\n'), '> q\n\n');
  assert.equal(quoteInsertion('Thanks', '> q\n\n'), '\n> q\n\n');
  assert.equal(quoteInsertion('Thanks\n', '> q\n\n'), '> q\n\n');
});

test('a code block has one copy button and collapses past twenty lines', () => {
  const block = (n: number) =>
    '```\n' +
    Array.from({ length: n }, (_, i) => `line ${i + 1}`).join('\n') +
    '\n```';
  const short = messageMarkup(block(20));
  assert.match(
    short,
    /<button type="button" class="chat-code-copy" aria-label="Copy code" title="Copy code">/,
  );
  assert.doesNotMatch(short, /role="status"|chat-code-more/);
  assert.match(short, /line 20<\/code>/);
  const long = messageMarkup(block(21));
  assert.match(long, /line 12<\/code><\/pre>/);
  assert.doesNotMatch(long, /line 13/);
  assert.match(
    long,
    /<button type="button" class="chat-code-more" aria-controls="([^"]+)" aria-expanded="false">.*Show all 21 lines<\/button>/,
  );
  const controls = /aria-controls="([^"]+)"/.exec(long)?.[1];
  assert.ok(controls && long.includes(`<pre id="${controls}">`));
});

test('a long code block is drawn whole while the hidden lines hold the find query', () => {
  const text =
    '```\n' +
    Array.from({ length: 25 }, (_, i) => `line ${i + 1}`).join('\n') +
    '\n```';
  const drawn = (holdsQuery: ((text: string) => boolean) | null) =>
    renderToStaticMarkup(
      createElement(ToastProvider, {
        controller: new ToastController(),
        children: createElement(MessageFindContext.Provider, {
          value: holdsQuery,
          children: createElement(MessageText, { text, actions }),
        }),
      }),
    );
  const whole = drawn((lines) => lines.includes('line 15'));
  assert.match(whole, /line 25<\/code>/);
  assert.match(whole, /aria-expanded="true">.*Show fewer lines<\/button>/);
  // A query the hidden lines do not hold, or none, leaves it collapsed.
  for (const holdsQuery of [() => false, null])
    assert.match(drawn(holdsQuery), /line 12<\/code><\/pre>/);
});

test('find matches the drawn text of a message as an older-page scan matches its source', () => {
  const mentions = chatMentions(['sam'], 'sam');
  const sources = [
    '**alpha** beta [docs](https://example.com/x) `code`',
    'The **can**ary is @sam, [bank.example](https://evil.example/)',
    '> quoted one\n> quoted two\n\n- item one\n1. item two',
    'see https://example.com/a. *done*\n```\nfn canary()\n```',
    '```\nunclosed **fence**',
  ];
  const needles = [
    'alpha beta',
    'example.com',
    '**',
    'canary',
    'one quoted',
    'quoted two',
    'item',
    '@sam',
    'mentions you',
    'bank.example',
    'evil',
    'fence',
    'docs code',
  ];
  for (const text of sources) {
    const { document } = new JSDOM(
      `<article data-message="m">${messageMarkup(text, mentions)}</article>`,
    ).window;
    const plain = find.foldFindText(
      find.findableText({
        id: 'm',
        sequence: '1',
        sender: null,
        send_time: '1',
        insert_time: '1',
        content: { kind: 'text', text },
      }) ?? '',
    );
    for (const needle of needles)
      assert.equal(
        find.findRanges(document.body, needle).length,
        find.findOffsets(plain, find.foldFindText(needle)).length,
        `${needle} in ${text}`,
      );
  }
});

test('@names on the roster are drawn as mentions, the reader as a mention of them', () => {
  const mentions = chatMentions(['satoshi', 'priya.n', 'łukasz'], 'satoshi');
  const html = messageMarkup(
    '@satoshi and @Priya.N. ask @nobody, mail bob@priya.n, @łukasz\n- @priya_n',
    mentions,
  );
  assert.match(
    html,
    /<p><span class="chat-mention self" title="Mentions you">@satoshi<span class="offscreen">, mentions you<\/span><\/span> and <span class="chat-mention">@Priya\.N<\/span>\. ask @nobody, mail bob@priya\.n, <span class="chat-mention">@łukasz<\/span><\/p>/,
  );
  assert.match(html, /<li><span class="chat-mention">@priya_n<\/span><\/li>/);
  // Letters FOKS flattens to ASCII compare equal in either direction.
  const flattened = messageMarkup(
    '@lukasz @Łukasz @straus @strauß',
    chatMentions(['Łukasz', 'straus'], null),
  );
  assert.equal((flattened.match(/class="chat-mention"/g) ?? []).length, 4);
  assert.equal(mentionKey('Łukasz'), mentionKey('lukasz'));
  assert.equal(mentionKey('strauß'), mentionKey('Straus'));
  assert.equal(mentionKey('bjørn.æ'), 'bjorn_a');
  // Without the roster, @names are text.
  assert.doesNotMatch(messageMarkup('@satoshi'), /chat-mention/);
  // A mention is text styling only: code and link labels keep their text.
  assert.doesNotMatch(
    messageMarkup('`@satoshi` [@satoshi](https://example.com/)', mentions),
    /chat-mention/,
  );
});

test('oversized line sets fall back to literal source', () => {
  const text = '**x**\n'.repeat(513);
  const html = messageMarkup(text);
  assert.doesNotMatch(html, /<strong>/);
  assert.match(html, /\*\*x\*\*/);
});

test('notification visibility requires focused intersection, not merely a loaded message', async () => {
  const { installDom } = await import('./lib/dom-harness');
  installDom({
    url: 'http://localhost/',
    body: '<div data-chat-store="s" data-chat-channel="a"><article data-message="m"></article></div>',
  });
  const { messageVisible } = await import('../src/chat/visibility');
  Object.defineProperty(document, 'hasFocus', {
    value: () => true,
    configurable: true,
  });
  const clip = document.querySelector<HTMLElement>('[data-chat-channel]')!;
  const item = document.querySelector<HTMLElement>('[data-message]')!;
  clip.getBoundingClientRect = () =>
    ({
      top: 0,
      bottom: 100,
      left: 0,
      right: 100,
      width: 100,
      height: 100,
    }) as DOMRect;
  item.getBoundingClientRect = () =>
    ({
      top: 101,
      bottom: 120,
      left: 0,
      right: 50,
      width: 50,
      height: 19,
    }) as DOMRect;
  assert.equal(messageVisible('s', 'a', 'm'), false);
  item.getBoundingClientRect = () =>
    ({
      top: 10,
      bottom: 30,
      left: 0,
      right: 50,
      width: 50,
      height: 20,
    }) as DOMRect;
  assert.equal(messageVisible('s', 'a', 'm'), true);
  assert.equal(messageVisible('other', 'a', 'm'), false);
  Object.defineProperty(document, 'hasFocus', {
    value: () => false,
    configurable: true,
  });
  assert.equal(messageVisible('s', 'a', 'm'), false);
});
