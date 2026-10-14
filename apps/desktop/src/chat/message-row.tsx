import type { ReactNode } from 'react';
import { AccountMark } from '../screens/account-switcher';
import {
  quoteReply,
  useMessageActions,
  type MessageActionOptions,
} from './message-actions';
import { MessageText } from './message-text';
import { messageClock, messageDate, messageTime } from './presentation';
import type { ThreadRow } from './thread-rows';
import { useChatConversationContext } from './conversation-provider';

/**
 * The structure every message of a conversation shares, whether history
 * holds it or it is still on its way: the avatar, a header with the author
 * and the time, the content, and a footer for a delivery status.
 *
 * Every row draws its time with {@link messageClock}; a relative age, if any,
 * belongs in `timeTitle`. A grouped row continues the previous message's run:
 * its avatar, author and time are kept for screen readers only, and anything
 * in `status` stays visible. The row's message actions
 * ({@link useMessageActions}) draw a toolbar over its top corner.
 */
export function MessageRow({
  className,
  grouped,
  avatarName,
  author,
  own,
  authorTitle,
  at,
  timeTitle,
  status,
  footer,
  attributes,
  actions = {},
  children,
}: {
  /** Classes beside `chat-message` and `grouped`. */
  className?: string;
  grouped: boolean;
  avatarName: string;
  author: string;
  own: boolean;
  authorTitle?: string;
  /** The row's time in milliseconds since the epoch. */
  at: string;
  /** The time's tooltip, or `null` to draw no time, as while sending. */
  timeTitle: string | null;
  /** Shown after the time: a sending spinner, a state, an action. */
  status?: ReactNode;
  /** Shown below the content: a delivery failure and its actions. */
  footer?: ReactNode;
  attributes?: Partial<Record<`data-${string}`, string>>;
  /** What the row's message actions offer; none without it. */
  actions?: MessageActionOptions;
  children: ReactNode;
}): ReactNode {
  const { bridge } = useChatConversationContext();
  const {
    open,
    onContextMenu,
    onKeyDown,
    onFocus,
    actions: tools,
  } = useMessageActions(
    {
      label: `Actions for ${own ? 'your message' : `${author}'s message`} at ${messageClock(at)}`,
      ...actions,
    },
    bridge,
  );
  return (
    <article
      className={[
        'chat-message',
        className,
        grouped && 'grouped',
        open && 'actions-open',
      ]
        .filter(Boolean)
        .join(' ')}
      {...attributes}
      title={grouped ? messageTime(at) : undefined}
      onContextMenu={onContextMenu}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
    >
      <AccountMark name={avatarName} className="chat-avatar" />
      <div className="chat-message-body">
        <div className="chat-message-content">
          <header>
            <span
              className={['chat-sender', own && 'you', grouped && 'offscreen']
                .filter(Boolean)
                .join(' ')}
              title={authorTitle}
            >
              {author}
            </span>
            {timeTitle !== null && (
              <time
                className={grouped ? 'offscreen' : undefined}
                dateTime={messageDate(at)?.toISOString()}
                title={timeTitle}
              >
                {messageClock(at)}
              </time>
            )}
            {status}
          </header>
          {children}
        </div>
        {footer}
      </div>
      {tools}
    </article>
  );
}

/** A message the history window holds. */
export function HistoryRow({
  row,
  onQuote,
}: {
  row: Extract<ThreadRow, { kind: 'message' }>;
  /** Inserts a quote into the composer; omitted where the reader cannot send. */
  onQuote?: (quote: string) => void;
}): ReactNode {
  const { bridge, mentions } = useChatConversationContext();
  const m = row.message;
  const text = m.content.kind === 'text' ? m.content.text : undefined;
  return (
    <MessageRow
      grouped={row.grouped}
      avatarName={row.avatarName}
      author={row.author}
      own={row.own}
      authorTitle={m.sender ?? undefined}
      at={m.insert_time}
      timeTitle={`Sent ${messageTime(m.send_time)} · Message #${m.sequence}`}
      attributes={{ 'data-message': m.id, 'data-sequence': m.sequence }}
      actions={{
        rowKey: row.key,
        text,
        onQuote:
          onQuote && text !== undefined
            ? () =>
                onQuote(
                  quoteReply(row.avatarName, m.insert_time, text, mentions),
                )
            : undefined,
        details: {
          sender: row.avatarName,
          own: row.own,
          // Only Basic text that decrypts and verifies is authenticated. An
          // oversized row is such text, which the agent does not return.
          unauthenticated:
            m.content.kind === 'unverifiable' ||
            m.content.kind === 'unsupported'
              ? m.content.kind
              : undefined,
          received: m.insert_time,
          sent: m.send_time,
          sequence: m.sequence,
          incomplete: row.incomplete,
        },
      }}
    >
      {m.content.kind === 'text' ? (
        <MessageText
          text={m.content.text}
          actions={bridge}
          mentions={mentions}
        />
      ) : (
        <p className="chat-unsupported">
          {m.content.kind === 'oversized'
            ? 'This message exceeds the desktop display limit.'
            : m.content.kind === 'unverifiable'
              ? 'This message could not be decrypted or verified.'
              : 'This message type is not supported yet.'}
        </p>
      )}
    </MessageRow>
  );
}
