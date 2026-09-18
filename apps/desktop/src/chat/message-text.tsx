import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { Button } from '../components/button';
import { ContextMenu, Menu } from '/kit/overlay-primitives';

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
  return (
    <>
      <a
        href={url}
        rel="noreferrer noopener"
        onClick={(event) => {
          event.preventDefault();
          void actions
            .openChatLink(url)
            .catch(() => setError('Could not open link.'));
        }}
      >
        {label}
      </a>
      {error && <span role="alert">{error}</span>}
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
  const [copyStatus, setCopyStatus] = useState('');
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
          event.preventDefault();
          setMenuPoint({ x: event.clientX, y: event.clientY });
        }}
      >
        {blocks.length ? blocks : <p>{text}</p>}
        <span className="offscreen" role="status">
          {copyStatus}
        </span>
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
                  .then(() => setCopyStatus('Copied'))
                  .catch(() => setCopyStatus('Could not copy'));
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
