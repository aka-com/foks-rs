import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import { createServer, type ViteDevServer } from 'vite';
let vite: ViteDevServer;
let MessageText: typeof import('../src/chat/message-text').MessageText;
let safeChatLink: typeof import('../src/chat/message-text').safeChatLink;
let labelHosts: typeof import('../src/chat/message-text').labelHosts;
let chatLinkDisplay: typeof import('../src/chat/message-text').chatLinkDisplay;
let relativeMessageTime: typeof import('../src/chat/presentation').relativeMessageTime;
let ToastProvider: typeof import('../kit/toasts').ToastProvider;
let ToastController: typeof import('../kit/toasts').ToastController;
test.before(async () => {
  vite = await createServer({
    configFile: new URL('../vite.config.ts', import.meta.url).pathname,
    appType: 'custom',
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  });
  ({ MessageText, safeChatLink, labelHosts, chatLinkDisplay } =
    await vite.ssrLoadModule('/src/chat/message-text.tsx'));
  ({ relativeMessageTime } = await vite.ssrLoadModule(
    '/src/chat/presentation.ts',
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
const messageMarkup = (text: string): string =>
  renderToStaticMarkup(
    createElement(ToastProvider, {
      controller: new ToastController(),
      children: createElement(MessageText, { text, actions }),
    }),
  );
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
    /<bdi dir="ltr" class="chat-link-host mismatch">\(evil\.example\)<\/bdi>/,
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

test('a plain-word label shows the destination host without a warning', () => {
  const html = messageMarkup('Notes are [here](https://docs.example/notes).');
  assert.match(html, /title="https:\/\/docs\.example\/notes"/);
  assert.match(
    html,
    />here<\/bdi><\/a><bdi dir="ltr" class="chat-link-host">\(docs\.example\)<\/bdi>/,
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
    ['https://user@bank.example', ['bank.example']],
    ['alice@bank.example', ['bank.example']],
    // A file or product name reads as a host only with a scheme, path, port
    // or www. prefix.
    ['index.html', []],
    ['Node.js', []],
    ['config.yaml', []],
    ['https://index.html', ['index.html']],
    ['index.html/x', ['index.html']],
    // Extensions that are also top-level domains are compared as hosts.
    ['README.md', ['readme.md']],
    ['docs.rs', ['docs.rs']],
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
    /<bdi>\u05e9\u05dc\u05d5\u05dd<\/bdi><\/a><bdi dir="ltr" class="chat-link-host">\(evil\.example\)<\/bdi>/,
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
