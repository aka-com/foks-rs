import {
  createContext,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react';
import type {
  FocusEvent,
  KeyboardEvent,
  MouseEvent,
  ReactNode,
  RefObject,
} from 'react';
import type { Bridge } from '../bridge';
import { Icon } from '../components';
import {
  anyDialogOpen,
  ContextMenu,
  Menu,
  Popover,
} from '/kit/overlay-primitives';
import { useOptionalToast } from '/kit/toasts';
import { mentionKey, type ChatMentions } from './mentions';
import { messageClock, messageTimeWithSeconds } from './presentation';

/** What Details lists for a message the history window holds. */
export interface MessageDetails {
  /** The sender's name. */
  sender: string;
  own: boolean;
  /**
   * Set when the sender, times and sequence are the server's unauthenticated
   * claims: `unverifiable` for a message that could not be decrypted or
   * verified, `unsupported` for a message type the client does not open.
   * Only Basic text that decrypts and verifies is authenticated.
   */
  unauthenticated?: 'unverifiable' | 'unsupported';
  /** The server's receive time, in milliseconds since the epoch. */
  received: string;
  /** The sender's clock when it sent the message. */
  sent: string;
  sequence: string;
  /**
   * The history page that loaded the message reported earlier messages it
   * could not check. This is a property of that page, not a result for the
   * message itself.
   */
  incomplete: boolean;
}

export interface MessageActionOptions {
  /** The row's key in the list's {@link ToolbarTabStop}. */
  rowKey?: string;
  /** The toolbar's and the menu's accessible name. */
  label?: string;
  /** The text Copy message copies; omitted for content without text. */
  text?: string;
  /** Writes a quote of the message into the composer; omitted where the reader cannot send. */
  onQuote?: () => void;
  /** Omitted for a message history does not hold yet. */
  details?: MessageDetails;
}

/**
 * The message list's one toolbar tab stop: the key of the row whose toolbar
 * Tab reaches, and the call a row makes when focus enters it. Other rows'
 * toolbars are reached by hover, the context menu, or focus entering the
 * row. Without a provider every toolbar is a tab stop.
 */
export interface ToolbarTabStop {
  current: string | null;
  claim: (rowKey: string) => void;
}
export const ToolbarTabStopContext = createContext<ToolbarTabStop | null>(null);

// A quote carries the first lines of a message, each cut at a word boundary
// near this many characters, so a long message is not pasted back whole.
const QUOTE_LINES = 2;
const QUOTE_LINE_CHARS = 100;

/**
 * A quote reply written as Basic text: a `>` header line naming the author
 * and the message's clock time, then the message's first lines as `>` lines,
 * and a blank line for the reply. Fence lines are left out, and a line that
 * is itself a quote loses its marker, so the quote is one level deep. The
 * author is written `@name` when the name is on the team's roster.
 */
export function quoteReply(
  author: string,
  at: string,
  text: string,
  mentions?: ChatMentions,
): string {
  const name = mentions?.members.has(mentionKey(author))
    ? `@${author}`
    : author;
  const source = text
    .split(/\r?\n/)
    .filter((line) => !line.startsWith('```'))
    .map((line) => line.replace(/^>\s?/, ''))
    .filter((line) => line.trim());
  // A line that was cut ends in an ellipsis, and so does the last line when
  // lines after it were left out.
  const lines = source.slice(0, QUOTE_LINES).map((line, index) => {
    const chars = Array.from(line.trimEnd());
    const dropped = index === QUOTE_LINES - 1 && source.length > QUOTE_LINES;
    if (chars.length <= QUOTE_LINE_CHARS)
      return dropped ? `${chars.join('')}…` : chars.join('');
    const head = chars.slice(0, QUOTE_LINE_CHARS - 1).join('');
    const space = head.lastIndexOf(' ');
    return `${space > QUOTE_LINE_CHARS / 2 ? head.slice(0, space) : head}…`;
  });
  return `${[`**${name}** · ${messageClock(at)}`, ...lines]
    .map((line) => `> ${line}`)
    .join('\n')}\n\n`;
}

/**
 * What goes into a draft at a caret after `before` to add `quote`: a line
 * break first when the caret is not at the start of a line.
 */
export function quoteInsertion(before: string, quote: string): string {
  return before && !before.endsWith('\n') ? `\n${quote}` : quote;
}

/**
 * The actions of one message row: a toolbar shown while the row is hovered or
 * holds focus, the same actions as a menu from a right-click, the ContextMenu
 * key or Shift+F10 within the row, and the Details popover. The row spreads
 * `onContextMenu`, `onKeyDown` and `onFocus` on its element and draws
 * `actions` inside it; `open` is set while the menu or Details is open, which
 * keeps the toolbar shown. Inside a {@link ToolbarTabStopContext} only the
 * current row's toolbar is a tab stop.
 *
 * Every action works on data the row already holds. Edit, Delete, Reaction
 * and Reply message types are reserved by the protocol but the server accepts
 * only Basic sends, so no action for them is drawn.
 */
export function useMessageActions(
  {
    rowKey,
    label: name = 'Message actions',
    text,
    onQuote,
    details,
  }: MessageActionOptions,
  bridge: Pick<Bridge, 'copyText'>,
): {
  open: boolean;
  onContextMenu: (event: MouseEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  onFocus: (event: FocusEvent<HTMLElement>) => void;
  actions: ReactNode;
} {
  const toasts = useOptionalToast();
  const stop = useContext(ToolbarTabStopContext);
  const tabbable = !stop || rowKey === undefined || stop.current === rowKey;
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  // The toolbar is one tab stop; the arrow keys move between its buttons.
  const [current, setCurrent] = useState(0);
  const detailsButton = useRef<HTMLButtonElement>(null);
  // Where focus was when the menu opened, so dismissing it returns there.
  const menuReturn = useRef<HTMLElement | null>(null);

  const copy = () => {
    if (text === undefined) return;
    void bridge
      .copyText(text)
      .then(() => toasts?.show('Copied'))
      .catch(() => toasts?.show('Could not copy', { tone: 'warning' }));
  };
  const closeMenu = (restore: boolean) => {
    setMenu(null);
    const target = menuReturn.current;
    menuReturn.current = null;
    if (restore && target?.isConnected && !anyDialogOpen())
      target.focus({ preventScroll: true });
  };
  const openMenu = (
    event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>,
    x: number,
    y: number,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    const active = document.activeElement;
    menuReturn.current =
      active instanceof HTMLElement && active !== document.body ? active : null;
    setMenu({ x, y });
  };
  // Menus, sheets and the Details popover are portaled out of the row, but
  // React still bubbles their events here; only the row's own content opens
  // the menu.
  const fromRow = (
    event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>,
  ) =>
    event.target instanceof Node && event.currentTarget.contains(event.target);

  // The toolbar and the menu draw the same list. `restore` returns focus to
  // where it was when the menu closes, for an action that does not move it.
  const tools: {
    label: string;
    icon: 'copy' | 'quote' | 'info';
    run: (fromMenu: boolean) => void;
    restore: boolean;
    popup?: boolean;
  }[] = [];
  if (text !== undefined)
    tools.push({
      label: 'Copy message',
      icon: 'copy',
      run: copy,
      restore: true,
    });
  if (onQuote)
    tools.push({
      label: 'Quote reply',
      icon: 'quote',
      run: onQuote,
      restore: false,
    });
  if (details)
    tools.push({
      label: 'Message details',
      icon: 'info',
      // The toolbar button toggles the popover; the menu entry opens it.
      run: (fromMenu) => setDetailsOpen((open) => fromMenu || !open),
      restore: false,
      popup: true,
    });
  const tabStop = Math.min(current, tools.length - 1);

  const available = tools.length > 0;
  const actions = available ? (
    <>
      <div
        className="chat-actions"
        role="toolbar"
        aria-label={name}
        onKeyDown={(event) => {
          const keys = ['ArrowLeft', 'ArrowRight', 'Home', 'End'];
          if (!keys.includes(event.key)) return;
          const items = Array.from(
            event.currentTarget.querySelectorAll<HTMLButtonElement>('button'),
          );
          const at = items.indexOf(document.activeElement as HTMLButtonElement);
          const next =
            event.key === 'Home'
              ? 0
              : event.key === 'End'
                ? items.length - 1
                : Math.min(
                    Math.max(at + (event.key === 'ArrowLeft' ? -1 : 1), 0),
                    items.length - 1,
                  );
          event.preventDefault();
          items[next]?.focus();
        }}
      >
        {tools.map(({ label, icon, run, popup }, index) => (
          <button
            type="button"
            key={label}
            ref={popup ? detailsButton : undefined}
            aria-label={label}
            title={label}
            aria-haspopup={popup ? 'dialog' : undefined}
            aria-expanded={popup ? detailsOpen : undefined}
            tabIndex={tabbable && index === tabStop ? 0 : -1}
            onFocus={() => setCurrent(index)}
            onClick={() => run(false)}
          >
            <Icon name={icon} />
          </button>
        ))}
      </div>
      {menu && (
        <ContextMenu
          point={menu}
          className="menu-portal"
          onClose={() => closeMenu(true)}
        >
          <Menu
            className="menu"
            aria-label={name}
            initialFocus="first"
            onClose={() => closeMenu(true)}
          >
            {tools.map(({ label, icon, run, restore }) => (
              <button
                type="button"
                key={label}
                onClick={() => {
                  closeMenu(restore);
                  run(true);
                }}
              >
                <Icon name={icon} />
                {label}
              </button>
            ))}
          </Menu>
        </ContextMenu>
      )}
      {detailsOpen && details && (
        <DetailsPopover
          details={details}
          anchor={detailsButton}
          onClose={(restore) => {
            setDetailsOpen(false);
            if (restore && !anyDialogOpen())
              detailsButton.current?.focus({ preventScroll: true });
          }}
        />
      )}
    </>
  ) : null;

  return {
    open: menu !== null || detailsOpen,
    onContextMenu: (event) => {
      if (!available || !fromRow(event)) return;
      openMenu(event, event.clientX, event.clientY);
    },
    // The keyboard opens the same menu from anything focused in the row, at
    // that element's corner.
    onKeyDown: (event) => {
      if (
        event.key !== 'ContextMenu' &&
        !(event.key === 'F10' && event.shiftKey)
      )
        return;
      if (!available || !fromRow(event)) return;
      const bounds = (event.target as HTMLElement).getBoundingClientRect();
      openMenu(event, bounds.left, bounds.bottom);
    },
    onFocus: () => {
      if (rowKey !== undefined) stop?.claim(rowKey);
    },
    actions,
  };
}

/**
 * A message's sender, its two times and its sequence, and the check of the
 * history page that loaded it. The agent reports verification for a page,
 * not for each message, so the wording names the page.
 */
function DetailsPopover({
  details,
  anchor,
  onClose,
}: {
  details: MessageDetails;
  anchor: RefObject<HTMLButtonElement | null>;
  /** `restore` is set when focus should return to the Details button. */
  onClose: (restore: boolean) => void;
}): ReactNode {
  const titleId = useId();
  const close = useRef<HTMLButtonElement>(null);
  // Opened from the toolbar or the menu, the popover takes focus, so the
  // keyboard reads it and Esc returns to the Details button.
  useEffect(() => {
    close.current?.focus({ preventScroll: true });
  }, []);
  const warn = (text: string) => ({
    tone: 'warn',
    icon: 'alert' as const,
    text,
  });
  const check =
    details.unauthenticated === 'unverifiable'
      ? warn('This message could not be decrypted or verified.')
      : details.unauthenticated === 'unsupported'
        ? warn(
            'This message type is not supported, so its sender and times are not verified.',
          )
        : details.incomplete
          ? warn(
              'Earlier messages on the history page that loaded this message could not be checked.',
            )
          : {
              tone: 'ok',
              icon: 'shield' as const,
              text: 'The history page that loaded this message was checked.',
            };
  return (
    <Popover
      anchorRef={anchor}
      className="menu-portal chat-details"
      align="end"
      onClose={() => onClose(false)}
    >
      <div role="dialog" aria-labelledby={titleId}>
        <div className="chat-details-head">
          <h3 id={titleId}>Message details</h3>
          <button
            type="button"
            ref={close}
            aria-label="Close details"
            title="Close"
            onClick={() => onClose(true)}
          >
            <Icon name="close" />
          </button>
        </div>
        <dl>
          <div>
            <dt>{details.unauthenticated ? 'Claimed sender' : 'Sent by'}</dt>
            <dd>
              {details.sender}
              {details.own && <small> (you)</small>}
            </dd>
          </div>
          <div>
            <dt>Received</dt>
            <dd>
              {messageTimeWithSeconds(details.received)} <small>(server)</small>
            </dd>
          </div>
          <div>
            <dt>Sender clock</dt>
            <dd>{messageTimeWithSeconds(details.sent)}</dd>
          </div>
          <div>
            <dt>Sequence</dt>
            <dd>{details.sequence}</dd>
          </div>
        </dl>
        <p className={`chat-details-check ${check.tone}`}>
          <Icon name={check.icon} />
          <span>{check.text}</span>
        </p>
      </div>
    </Popover>
  );
}
