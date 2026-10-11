import { useId, useState } from 'react';
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

// A scheme as browsers accept it before a host: `https://`, and also
// `https:/` and `https:\\`.
const SCHEME = /[a-z][a-z\d+.-]*:[/\\]+/iu;
// A dotted name whose last label is two or more characters starting with a
// letter (`bank.example`, an IDN in Unicode or `xn--` form), or a dotted-quad
// IPv4 address, not preceded or followed by a letter, digit or hyphen. Any
// other character may border it, so quotes, brackets, `'s`, dashes and
// ellipses around a host do not hide it. Version numbers such as `1.5` and
// `v1.2.3`, and abbreviations such as `e.g`, do not match. The extra
// separators are the full stops the URL parser maps to `.`.
const DOTTED_HOST =
  /(?<![\p{L}\p{N}-])(?:(?:[\p{L}\p{N}-]+[.\u3002\uff0e\uff61])+\p{L}[\p{L}\p{N}-]+|\d{1,3}(?:\.\d{1,3}){3})(?![\p{L}\p{N}-])/gu;
// Final labels that make a bare name such as `index.html` or `Node.js` read
// as a file or product name rather than a host. Extensions that are also
// top-level domains (md, py, rs, sh, zip, mov and others) are left out, so
// `docs.rs` and `README.md` are compared as hosts.
const FILE_EXTENSIONS = new Set(
  (
    'cjs conf cpp css csv gif gz htm html ini jpeg jpg js json jsx kt lock ' +
    'log mjs mp3 mp4 pdf php png rb sql svg tar toml ts tsx txt wasm xml yaml ' +
    'yml'
  ).split(' '),
);

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

/**
 * The hosts one whitespace-separated word of a label names. After a scheme,
 * that is every dotted name in the authority (both sides of an `@`, since a
 * reader may take either for the host), or the name after the last `@` when
 * the authority has no dotted name.
 * Without a scheme, it is every dotted name before the first `/`, `\`, `?` or
 * `#`, except a path-less file or product name.
 */
function wordHosts(word: string): string[] {
  const scheme = SCHEME.exec(word);
  const rest = scheme ? word.slice(scheme.index + scheme[0].length) : word;
  const authority = rest.split(/[/?#\\]/u, 1)[0];
  const hosts: string[] = [];
  for (const [name] of authority.matchAll(DOTTED_HOST)) {
    const last = name.slice(name.search(/[^.\u3002\uff0e\uff61]+$/u));
    if (
      !scheme &&
      rest === authority &&
      FILE_EXTENSIONS.has(last.toLowerCase())
    )
      continue;
    hosts.push(normalHost(name));
  }
  if (scheme) {
    // A scheme URL names a host even without a dot, as in `https://intranet`.
    const named = /^[\p{L}\p{N}-]+/u.exec(
      authority.slice(authority.lastIndexOf('@') + 1),
    );
    if (named && !hosts.length) hosts.push(normalHost(named[0]));
  }
  return hosts;
}

/**
 * The hosts a link label names, in the same form as `linkHost`, from every
 * word that reads as a URL (`scheme://…`) or contains a host name
 * (`bank.example/login`, `"bank.example"`). A label such as "here" names
 * none. Invisible format characters (Unicode category Cf, such as U+200B ZERO
 * WIDTH SPACE or bidirectional overrides) are removed first, so they cannot
 * split a host name out of the match. The whole label is read; its length is
 * bounded by the message limits `MessageText` applies.
 */
export function labelHosts(label: string): string[] {
  const hosts = new Set<string>();
  for (const word of label.replace(/\p{Cf}/gu, '').split(/\s+/u))
    for (const host of wordHosts(word)) hosts.add(host);
  return [...hosts];
}

/** Hosts that differ only by one leading `www.` are treated as the same. */
function sameHost(a: string, b: string): boolean {
  return a.replace(/^www\./, '') === b.replace(/^www\./, '');
}

export interface ChatLinkDisplay {
  /** The destination host, from `linkHost`. */
  host: string;
  /** False only when the label is the destination URL itself. */
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
  const text = label.trim();
  return {
    host,
    showHost: text !== href && `${text}/` !== href,
    mismatchedHost:
      labelHosts(label).find((named) => !sameHost(named, host)) ?? null,
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
      <p id={descriptionId}>
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
  const { host, showHost, mismatchedHost } = chatLinkDisplay(label, url);
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
        {/* Isolated so a bidirectional override in the label cannot reorder
            the host shown after the link. */}
        <bdi>{label}</bdi>
      </a>
      {showHost && (
        <span
          className={
            mismatchedHost ? 'chat-link-host mismatch' : 'chat-link-host'
          }
        >
          ({host})
        </span>
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
