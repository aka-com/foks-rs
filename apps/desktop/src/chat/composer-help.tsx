import { useEffect, useId, useRef } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Icon } from '../components';
import { Popover } from '/kit/overlay-primitives';
import { commandKey } from './composer-format';
import { useChatConversationContext } from './conversation-provider';
import { MessageText } from './message-text';

/**
 * What the composer's formatting help lists, one row per form
 * `message-text.tsx` renders. Each example is drawn by `MessageText`, so the
 * sheet shows what a sent message shows and cannot list a form it does not
 * render.
 */
export const FORMATTING_EXAMPLES: readonly {
  source: string;
  note?: string;
}[] = [
  { source: '**bold**' },
  { source: '*italic*' },
  { source: '`code`' },
  { source: '[notes](https://example.com)' },
  { source: 'https://example.com' },
  { source: '```\nkubectl get pods\n```', note: 'with a Copy code button' },
  { source: '- item', note: 'or * item' },
  { source: '1. item' },
  { source: '> quote' },
];

/**
 * The composer's formatting and key help, opened from the ? beside the hint
 * line. It takes focus when it opens; Esc or Close returns focus to the ?.
 */
export function ComposerHelp({
  anchor,
  onClose,
}: {
  anchor: RefObject<HTMLButtonElement | null>;
  /** `restore` is set when focus should return to the ? button. */
  onClose: (restore: boolean) => void;
}): ReactNode {
  const { bridge } = useChatConversationContext();
  const titleId = useId();
  const keysId = useId();
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    close.current?.focus({ preventScroll: true });
  }, []);
  return (
    <Popover
      anchorRef={anchor}
      className="menu-portal chat-details chat-format-help"
      align="end"
      onClose={() => onClose(false)}
    >
      <div role="dialog" aria-labelledby={titleId}>
        <div className="chat-details-head">
          <h3 id={titleId}>Formatting and keys</h3>
          <button
            type="button"
            ref={close}
            aria-label="Close formatting help"
            title="Close"
            onClick={() => onClose(true)}
          >
            <Icon name="close" />
          </button>
        </div>
        <table>
          <thead>
            <tr>
              <th scope="col">Type</th>
              <th scope="col">Shows as</th>
            </tr>
          </thead>
          <tbody>
            {FORMATTING_EXAMPLES.map(({ source, note }) => (
              <tr key={source}>
                <td>
                  <code>{source}</code>
                </td>
                {/* A sample, not a message: its link and copy button are
                    drawn but do not act. */}
                <td inert>
                  <MessageText text={source} actions={bridge} />
                  {note && <small>{note}</small>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="chat-format-note">
          Links open only for http and https addresses. Styles do not nest or
          run across lines. Other Markdown, such as # headings or _underscores_,
          shows as typed.
        </p>
        <h4 id={keysId}>Keys</h4>
        <dl aria-labelledby={keysId}>
          <div>
            <dt>
              <kbd>Enter</kbd>
            </dt>
            <dd>Send</dd>
          </div>
          <div>
            <dt>
              <kbd>Shift+Enter</kbd>
            </dt>
            <dd>New line (Alt+Enter also works)</dd>
          </div>
          <div>
            <dt>
              <kbd>{commandKey('B')}</kbd> <kbd>{commandKey('I')}</kbd>{' '}
              <kbd>{commandKey('E')}</kbd>
            </dt>
            <dd>Bold, italic, code; again to remove</dd>
          </div>
          <div>
            <dt>
              <kbd>{commandKey('E', true)}</kbd>
            </dt>
            <dd>Code block</dd>
          </div>
          <div>
            <dt>
              <kbd>@</kbd>
            </dt>
            <dd>Mention a team member</dd>
          </div>
          <div>
            <dt>
              <kbd>↑</kbd>
            </dt>
            <dd>Edit your newest unsent message, when the box is empty</dd>
          </div>
        </dl>
      </div>
    </Popover>
  );
}
