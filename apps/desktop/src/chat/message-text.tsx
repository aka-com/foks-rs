import { useId, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { Button } from '../components/button';
import { Icon } from '../components/icon';
import { SheetDialog } from '../components/sheet';
import { ContextMenu, Menu } from '/kit/overlay-primitives';
import { useOptionalToast } from '/kit/toasts';

/** Literal absolute HTTP(S) only; never decode entities into executable URLs. */
export function safeChatLink(value: string): string | null {
  if (
    value.length > 2048 ||
    Array.from(value).some(
      (c) =>
        /[\s\\]/u.test(c) || c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127,
    )
  )
    return null;
  if (!/^https?:\/\//i.test(value)) return null;
  try {
    const url = new URL(value);
    return url.hostname && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}

type Actions = Pick<Bridge, 'copyText' | 'openChatLink'>;
function Copy({
  text,
  label,
  actions,
}: {
  text: string;
  label: string;
  actions: Actions;
}) {
  const [status, setStatus] = useState('');
  return (
    <>
      <Button
        size="sm"
        onClick={() => {
          void actions
            .copyText(text)
            .then(() => setStatus('Copied'))
            .catch(() => setStatus('Could not copy'));
        }}
      >
        {label}
      </Button>
      <span role="status">{status}</span>
    </>
  );
}

/**
 * The destination host as the URL parser reports it: lower case, with
 * internationalized labels in their `xn--` form, so a lookalike host is shown
 * as the ASCII name it resolves to. A trailing root dot is dropped.
 */
export function linkHost(href: string): string {
  try {
    return new URL(href).hostname.replace(/\.$/, '');
  } catch {
    return '';
  }
}

// The full stops the URL parser maps to `.` in a host (U+3002, U+FF0E and
// U+FF61), and U+FE52 SMALL FULL STOP and U+2024 ONE DOT LEADER, whose
// compatibility mapping is `.`.
const STOPS = '.\\u3002\\uff0e\\uff61\\ufe52\\u2024';
// Letters, combining marks, digits and hyphens.
const LABEL_CHARS = '\\p{L}\\p{M}\\p{N}\\-';
const SEPARATOR = new RegExp(`[${STOPS}]`, 'u');
// Maximal runs of label characters and full stops; each is read once, so the
// cost of reading a label is linear in its length.
const NAME_RUN = new RegExp(`[${LABEL_CHARS}${STOPS}]+`, 'gu');
// The name at the start of an authority, dotted or not.
const LEADING_NAME = new RegExp(
  `^[\\p{L}\\p{N}\\-][${LABEL_CHARS}]*(?:[${STOPS}][${LABEL_CHARS}]+)*`,
  'u',
);
const MARKS = /\p{M}/gu;
// A keycap emoji (`1️⃣`), whose U+20E3 would otherwise join the digit to the
// name after it. Read as a space; variation selectors are already removed.
const KEYCAP = /[\d#*]\u20e3/gu;
// A last label: two or more characters starting with a letter (`example`, an
// IDN label in Unicode or `xn--` form), tested with combining marks removed.
const LAST_LABEL = /^\p{L}[\p{L}\p{N}-]+$/u;
const IPV4_PART = /^\d{1,3}$/;
// A name longer than this many UTF-16 code units, the longest link
// `safeChatLink` accepts, is reported shortened rather than passed to the URL
// parser, whose normalization takes quadratic time on a long run of combining
// marks. Even after the parser's mappings, such a name is longer than 253
// characters, the longest DNS name, so it names no host a link can reach.
const MAX_NAME = 2048;
const SHORTENED_NAME = 253;
// The end of an authority.
const AUTHORITY_END = /[/?#\\]/u;
// Characters that draw nothing: format characters (Unicode category Cf, such
// as U+200B ZERO WIDTH SPACE and bidirectional controls) and the other default
// ignorable code points (U+034F COMBINING GRAPHEME JOINER, variation
// selectors, U+180B..U+180D, U+17B4 and U+17B5). The Hangul fillers are
// default ignorable letters that many fonts draw as a blank, so they are kept.
const INVISIBLE = /(?!\p{L})[\p{Cf}\p{Default_Ignorable_Code_Point}]/gu;
// The bidirectional formatting characters: U+061C, U+200E, U+200F,
// U+202A..U+202E and U+2066..U+2069.
const BIDI_CONTROL = /\p{Bidi_Control}/gu;
// A scheme as browsers accept it before a host: `https://`, and also
// `https:/` and `https:\\`. A match can start only where a run of scheme
// characters starts, and the part before its first letter holds no letter, so
// a long word without a colon is scanned once rather than from every position.
const SCHEME = /(?<![a-z\d+.-])[\d+.-]*[a-z][a-z\d+.-]*:[/\\]+/iu;
// Final labels that make a bare name such as `index.html` or `Node.js` read
// as a file or product name rather than a host. The rule applies to the whole
// name, so `vite.config.ts` is not read as a host, and neither is
// `paypal.com.js`; the destination host shown after the link still names
// where it opens.
const FILE_EXTENSIONS = new Set(
  (
    'cjs conf cpp css csv gif gz htm html ini jpeg jpg js json jsx kt lock ' +
    'log mjs mp3 mp4 pdf php png rb sql svg tar toml ts tsx txt wasm xml yaml ' +
    'yml'
  ).split(' '),
);
// File extensions that are also top-level domains. A name ending in one is
// read as a file name only when its stem marks it as one (`README.md`,
// `setup_env.sh`, `Main.java`); `docs.rs` and `get.sh` are compared as hosts.
const TLD_FILE_EXTENSIONS = new Set('java md mov py rs sh zip'.split(' '));
// Runs of the characters a file name's stem is written with: label
// characters, full stops and underscores.
const STEM_RUN = new RegExp(`[${LABEL_CHARS}${STOPS}_]+`, 'gu');
// A character that marks a stem as part of a file name: an upper-case
// letter, or an underscore after a letter, mark or digit. A leading
// underscore, as in `_docs.rs_`, is emphasis rather than part of a name.
const STEM_MARK = /\p{Lu}|(?<=[\p{L}\p{M}\p{N}])_/u;
// `?@` or `#@` directly after an authority, or after one `/` or `\` that
// follows it, then an optional scheme, and the text up to the next `/`, `?`,
// `#` or `\`, which a reader who takes the host to follow the last `@` reads
// as the host (`bank.example#@evil.example`,
// `bank.example#@https://evil.example`).
const QUERY_AT = /^[/\\]?[?#]@(?:[a-z][a-z\d+.-]*:[/\\]+)?([^/?#\\]*)/iu;

function normalHost(host: string): string {
  try {
    return (
      new URL(`https://${host}`).hostname.replace(/\.$/, '') ||
      host.toLowerCase()
    );
  } catch {
    return host.toLowerCase();
  }
}

/** A name read from a label, as its labels, in the same form as `linkHost`. */
function hostName(labels: string[]): string {
  const name = labels.join('.');
  if (name.length <= MAX_NAME) return normalHost(name);
  const shown = [...name.slice(0, 2 * SHORTENED_NAME)].slice(0, SHORTENED_NAME);
  return `${shown.join('').toLowerCase()}…`;
}

function nameHost({ labels }: DottedName): string {
  return hostName(labels);
}

/**
 * The dotted names in one run of consecutive non-empty labels, each as the
 * start and end of its labels: the labels up to the last one shaped like a
 * final label (`bank.example`), then any four one- to three-digit labels (an
 * IPv4 address) after it. Version numbers such as `1.5` and `v1.2.3`, and
 * abbreviations such as `e.g`, name nothing.
 */
function labelRunNames(labels: string[]): [number, number][] {
  const names: [number, number][] = [];
  let last = 0;
  for (let i = 1; i < labels.length; i++)
    if (LAST_LABEL.test(labels[i].replace(MARKS, ''))) last = i;
  let i = 0;
  if (last) {
    names.push([0, last + 1]);
    i = last + 1;
  }
  while (i + 4 <= labels.length) {
    let parts = 0;
    while (parts < 4 && IPV4_PART.test(labels[i + parts])) parts++;
    if (parts === 4) {
      names.push([i, i + 4]);
      i += 4;
    } else i++;
  }
  return names;
}

interface DottedName {
  labels: string[];
  /** Where the last label starts in the text the name was read from. */
  lastAt: number;
}

/**
 * The dotted names in `text`, in order. A name is bordered by any character
 * other than a letter, combining mark, digit, hyphen or full stop, so quotes,
 * brackets, `'s`, dashes and ellipses around a host do not hide it. An empty
 * label (a leading, trailing or doubled full stop) also ends a name.
 */
function dottedNames(text: string): DottedName[] {
  const names: DottedName[] = [];
  for (const run of text.matchAll(NAME_RUN)) {
    let labels: string[] = [];
    let starts: number[] = [];
    // Every full stop is one UTF-16 code unit.
    let at = run.index;
    for (const label of [...run[0].split(SEPARATOR), '']) {
      if (label) {
        labels.push(label);
        starts.push(at);
      } else {
        for (const [start, end] of labelRunNames(labels))
          names.push({
            labels: labels.slice(start, end),
            lastAt: starts[end - 1],
          });
        labels = [];
        starts = [];
      }
      at += label.length + 1;
    }
  }
  return names;
}

/**
 * A test of whether a dotted name read from the path-less word `word` is a
 * file or product name rather than a host. Its first label is not `www`, and
 * its last label is a file extension that is not a top-level domain, or one
 * that is and the stem before it, within one run of `STEM_RUN` characters,
 * holds a `STEM_MARK`. The test is applied to names in the order `dottedNames` gives them, and
 * reads `word` at most once.
 */
function fileNameTest(word: string): (name: DottedName) => boolean {
  let stems: { end: number; mark: number }[] | undefined;
  let stem = 0;
  return ({ labels, lastAt }) => {
    if (labels[0].toLowerCase() === 'www') return false;
    const extension = labels[labels.length - 1].toLowerCase();
    if (FILE_EXTENSIONS.has(extension)) return true;
    if (!TLD_FILE_EXTENSIONS.has(extension)) return false;
    stems ??= Array.from(word.matchAll(STEM_RUN), (run) => {
      const mark = run[0].search(STEM_MARK);
      return {
        end: run.index + run[0].length,
        mark: mark < 0 ? Infinity : run.index + mark,
      };
    });
    while (stems[stem].end <= lastAt) stem++;
    return stems[stem].mark < lastAt;
  };
}

/** The dotted names after a `?@` or `#@` that starts `tail` (`QUERY_AT`). */
function queryAtHosts(tail: string): string[] {
  const queryAt = QUERY_AT.exec(tail);
  return queryAt ? dottedNames(queryAt[1]).map(nameHost) : [];
}

/**
 * The hosts one whitespace-separated word of a label names. After a scheme,
 * that is every host named before the scheme (`paypal.com/x:/evil.example`),
 * then every dotted name in the authority (both sides of an `@`, since a
 * reader may take either for the host), or the name after the last `@` when
 * the authority has no dotted name, and every dotted name after a `?@` or
 * `#@` that ends the authority (`https://bank.example#@evil.example`).
 * Without a scheme, it is every dotted name before the first `/`, `\`, `?` or
 * `#`, except a file or product name (`fileNameTest`) in a word that is
 * nothing else, and, when there is one, every dotted name after a `?@` or
 * `#@` that ends it (`bank.example#@evil.example`).
 */
function wordHosts(word: string, wholeWord = true): string[] {
  const scheme = SCHEME.exec(word);
  if (!scheme) {
    const authority = word.split(AUTHORITY_END, 1)[0];
    const names = dottedNames(authority);
    if (authority === word && wholeWord) {
      const fileName = fileNameTest(word);
      return names.filter((name) => !fileName(name)).map(nameHost);
    }
    const hosts = names.map(nameHost);
    if (hosts.length) hosts.push(...queryAtHosts(word.slice(authority.length)));
    return hosts;
  }
  const rest = word.slice(scheme.index + scheme[0].length);
  const authority = rest.split(AUTHORITY_END, 1)[0];
  const hosts = dottedNames(authority).map(nameHost);
  if (!hosts.length) {
    // A scheme URL names a host even without a dotted name, as in
    // `https://intranet` or `https://127.1`.
    const named = LEADING_NAME.exec(
      authority.slice(authority.lastIndexOf('@') + 1),
    );
    if (named) hosts.push(hostName(named[0].split(SEPARATOR)));
  }
  hosts.push(...queryAtHosts(rest.slice(authority.length)));
  // The text before the scheme's `:` holds no scheme, since this one is the
  // leftmost, and is followed by the rest of the URL.
  const before = word.slice(0, scheme.index + scheme[0].indexOf(':'));
  return [...wordHosts(before, false), ...hosts];
}

/**
 * The hosts a link label names, in the same form as `linkHost`, from every
 * word that reads as a URL (`scheme://…`) or contains a host name
 * (`bank.example/login`, `"bank.example"`). A label such as "here" names
 * none. Invisible characters are removed first, so they cannot split a host
 * name out of the match. The whole label is read, in time linear in its
 * length.
 */
export function labelHosts(label: string): string[] {
  const hosts = new Set<string>();
  const text = label.replace(INVISIBLE, '').replace(KEYCAP, ' ');
  for (const word of text.split(/\s+/u))
    for (const host of wordHosts(word)) hosts.add(host);
  return [...hosts];
}

/** Hosts that differ only by one leading `www.` are treated as the same. */
function sameHost(a: string, b: string): boolean {
  return a.replace(/^www\./, '') === b.replace(/^www\./, '');
}

export interface ChatLinkDisplay {
  /**
   * The label as drawn: without bidirectional formatting characters, which
   * would make the order the reader sees differ from the order `labelHosts`
   * reads (`bank.\u202Eac` displays as `bank.ca`).
   */
  label: string;
  /** The destination host, from `linkHost`. */
  host: string;
  /**
   * False only when the label is the destination URL itself, up to the case
   * of ASCII letters and a final `/`, and names no other host.
   */
  showHost: boolean;
  /**
   * The first host the label names that differs from `host`. The link then
   * carries a warning and opens only from the Check this link sheet.
   */
  mismatchedHost: string | null;
}

/** How a `[label](url)` link is drawn; `href` is a `safeChatLink` result. */
export function chatLinkDisplay(label: string, href: string): ChatLinkDisplay {
  const host = linkHost(href);
  const drawn = label.replace(BIDI_CONTROL, '');
  // ASCII letters only: a non-ASCII letter that the URL parser maps to an
  // ASCII one leaves the host suffix in place.
  const text = drawn.trim().replace(/[A-Z]+/g, (upper) => upper.toLowerCase());
  const url = href.replace(/[A-Z]+/g, (upper) => upper.toLowerCase());
  const mismatchedHost =
    labelHosts(label).find((named) => !sameHost(named, host)) ?? null;
  return {
    label: drawn,
    host,
    showHost: mismatchedHost !== null || (text !== url && `${text}/` !== url),
    mismatchedHost,
  };
}

function LinkCheck({
  url,
  host,
  namedHost,
  onCopy,
  onOpen,
  onClose,
}: {
  url: string;
  host: string;
  namedHost: string;
  onCopy: () => void;
  onOpen: () => void;
  onClose: () => void;
}) {
  const descriptionId = useId();
  return (
    <SheetDialog
      danger
      describedBy={descriptionId}
      glyph={
        <span className="kico md warning">
          <Icon name="alert" />
        </span>
      }
      title="Check this link"
      onClose={onClose}
      footer={
        <>
          <Button icon="copy" onClick={onCopy}>
            Copy link
          </Button>
          <span className="spacer" />
          <Button icon="externalLink" onClick={onOpen}>
            Open anyway
          </Button>
          <Button
            variant="primary"
            data-dialog-autofocus="true"
            onClick={onClose}
          >
            Cancel
          </Button>
        </>
      }
    >
      <p id={descriptionId} className="chat-link-hosts">
        The link text names <b>{namedHost}</b>, but the link opens <b>{host}</b>
        .
      </p>
      <div className="chat-link-destination">
        <small>Opens</small>
        <code>{url}</code>
      </div>
    </SheetDialog>
  );
}

function Link({
  url,
  label,
  actions,
}: {
  url: string;
  label: string;
  actions: Actions;
}) {
  const [error, setError] = useState('');
  const [checking, setChecking] = useState(false);
  const toasts = useOptionalToast();
  const {
    label: drawn,
    host,
    showHost,
    mismatchedHost,
  } = useMemo(() => chatLinkDisplay(label, url), [label, url]);
  const open = () => {
    void actions
      .openChatLink(url)
      .catch(() => setError('Could not open link.'));
  };
  return (
    <>
      <a
        href={url}
        title={url}
        className={mismatchedHost ? 'chat-link-mismatch' : undefined}
        rel="noreferrer noopener"
        onClick={(event) => {
          event.preventDefault();
          if (mismatchedHost) setChecking(true);
          else open();
        }}
      >
        {mismatchedHost && (
          <>
            <Icon name="alert" />
            <span className="offscreen">
              Link text names a different host:{' '}
            </span>
          </>
        )}
        {/* Isolated so right-to-left text in the label cannot reorder the
            host shown after the link. */}
        <bdi>{drawn}</bdi>
        {showHost && <span className="offscreen">, opens {host}</span>}
      </a>
      {showHost && (
        // Isolated left to right, so a bidirectional control in the message
        // text before the link cannot reverse the host. The link's name
        // already states the host, so assistive technology skips this copy.
        <bdi
          dir="ltr"
          aria-hidden="true"
          className={
            mismatchedHost ? 'chat-link-host mismatch' : 'chat-link-host'
          }
        >
          ({host})
        </bdi>
      )}
      {error && <span role="alert">{error}</span>}
      {checking && mismatchedHost && (
        <LinkCheck
          url={url}
          host={host}
          namedHost={mismatchedHost}
          onClose={() => setChecking(false)}
          onCopy={() => {
            setChecking(false);
            void actions
              .copyText(url)
              .then(() => toasts?.show('Link copied'))
              .catch(() => toasts?.show('Could not copy', { tone: 'warning' }));
          }}
          onOpen={() => {
            setChecking(false);
            open();
          }}
        />
      )}
    </>
  );
}

function inline(text: string, actions: Actions): ReactNode[] {
  // Deliberately nonrecursive and bounded. Unsupported/nested syntax remains text.
  const parts: ReactNode[] = [];
  const pattern =
    /(`[^`\n]+`|\*\*[^*\n]+\*\*|\*[^*\n]+\*|\[[^\]\n]+\]\([^\s)]+\))/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    if (parts.length >= 256) break;
    const token = match[0],
      at = match.index;
    parts.push(text.slice(last, at));
    if (token.startsWith('`'))
      parts.push(<code key={at}>{token.slice(1, -1)}</code>);
    else if (token.startsWith('**'))
      parts.push(<strong key={at}>{token.slice(2, -2)}</strong>);
    else if (token.startsWith('*'))
      parts.push(<em key={at}>{token.slice(1, -1)}</em>);
    else {
      const split = token.indexOf('](');
      const url = safeChatLink(token.slice(split + 2, -1));
      parts.push(
        url ? (
          <Link
            key={at}
            url={url}
            label={token.slice(1, split)}
            actions={actions}
          />
        ) : (
          token
        ),
      );
    }
    last = at + token.length;
  }
  parts.push(text.slice(last));
  return parts;
}

export function MessageText({
  text,
  actions,
}: {
  text: string;
  actions: Actions;
}) {
  const [menuPoint, setMenuPoint] = useState<{ x: number; y: number } | null>(
    null,
  );
  const toasts = useOptionalToast();
  const lines = text.split('\n');
  const blocks: ReactNode[] = [];
  if (text.length <= 65536 && lines.length <= 512) {
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      if (line.startsWith('```')) {
        const start = i;
        const body: string[] = [];
        while (++i < lines.length && !lines[i].startsWith('```'))
          body.push(lines[i]);
        if (i === lines.length) {
          blocks.push(<p key={start}>{lines.slice(start).join('\n')}</p>);
          break;
        }
        const code = body.join('\n');
        blocks.push(
          <div key={start}>
            <pre>
              <code>{code}</code>
            </pre>
            <Copy text={code} label="Copy code" actions={actions} />
          </div>,
        );
      } else if (/^[-*] /.test(line) || /^\d+\. /.test(line)) {
        const start = i,
          ordered = /^\d+\. /.test(line),
          items: ReactNode[] = [];
        const marker = ordered ? /^\d+\. / : /^[-*] /;
        do {
          items.push(
            <li key={i}>{inline(lines[i].replace(marker, ''), actions)}</li>,
          );
          i++;
        } while (i < lines.length && marker.test(lines[i]));
        i--;
        blocks.push(
          ordered ? <ol key={start}>{items}</ol> : <ul key={start}>{items}</ul>,
        );
      } else if (line.startsWith('> '))
        blocks.push(
          <blockquote key={i}>{inline(line.slice(2), actions)}</blockquote>,
        );
      else if (line) blocks.push(<p key={i}>{inline(line, actions)}</p>);
    }
  }
  return (
    <>
      <div
        className="chat-message-text"
        onContextMenu={(event) => {
          // A link's Check this link sheet is portaled out of this element,
          // but React still bubbles its events here.
          if (!event.currentTarget.contains(event.target as Node)) return;
          event.preventDefault();
          setMenuPoint({ x: event.clientX, y: event.clientY });
        }}
      >
        {blocks.length ? blocks : <p>{text}</p>}
      </div>
      {menuPoint ? (
        <ContextMenu
          point={menuPoint}
          className="menu-portal"
          onClose={() => setMenuPoint(null)}
        >
          <Menu
            className="menu"
            initialFocus="first"
            onClose={() => setMenuPoint(null)}
          >
            <button
              type="button"
              onClick={() => {
                setMenuPoint(null);
                void actions
                  .copyText(text)
                  .then(() => toasts?.show('Copied'))
                  .catch(() =>
                    toasts?.show('Could not copy', { tone: 'warning' }),
                  );
              }}
            >
              Copy message
            </button>
          </Menu>
        </ContextMenu>
      ) : null}
    </>
  );
}
