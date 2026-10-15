import type { ChatMessage } from '../chat-contract';
import { CHAT_NOTIFICATION_TEXT_CHARS } from '../chat-limits';
import { ALERT_TITLE_CHARS } from './local-contract';
/** Select verified incoming Basic messages; positions and delivery are independent. */
export function incoming(
  messages: readonly ChatMessage[],
  after: bigint,
  upper: bigint,
  actor: string,
  visible: (id: string) => boolean,
): ChatMessage[] {
  const ids = new Set<string>();
  return messages.filter((m) => {
    const seq = BigInt(m.sequence);
    if (
      seq <= after ||
      seq > upper ||
      !m.sender ||
      m.sender === actor ||
      m.content.kind !== 'text' ||
      ids.has(m.id) ||
      visible(m.id)
    )
      return false;
    ids.add(m.id);
    return true;
  });
}
/**
 * The alerts a channel's new messages raise, given newest first: one per
 * message for the newest three, oldest of them first, then one that counts
 * the rest. The newest message is the last alert that carries a body, so a
 * platform that keeps one entry per channel ends on its preview. With
 * `sender`, a body starts with the name it gives the message's sender;
 * either way it is cut to `CHAT_NOTIFICATION_TEXT_CHARS`, the bound the
 * native side admits.
 */
export function notificationText(
  messages: readonly ChatMessage[],
  incomplete: boolean,
  sender?: (id: string | null) => string,
): { body?: string; count: number; incomplete: boolean }[] {
  const first = messages
    .slice(0, 3)
    .reverse()
    .map((m) => ({
      body:
        m.content.kind === 'text'
          ? Array.from(
              sender
                ? `${singleLine(sender(m.sender))}: ${m.content.text}`
                : m.content.text,
            )
              .slice(0, CHAT_NOTIFICATION_TEXT_CHARS)
              .join('')
          : undefined,
      count: 1,
      incomplete: false,
    }));
  if (messages.length > 3 || (incomplete && messages.length > 0))
    first.push({
      body: undefined,
      count: Math.max(0, messages.length - 3),
      incomplete,
    });
  return first;
}

/** Control characters, which a notification service may draw as breaks. */
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/g;
function singleLine(text: string): string {
  return text.replace(CONTROL, ' ');
}
/**
 * An alert's title: the channel, then its team when known, as in
 * "#general · Engineering". The team's name is cut with "…" to keep the
 * title within `ALERT_TITLE_CHARS`; control characters become spaces, since
 * the native side refuses a title that holds one.
 */
export function alertTitle(channel: string, team?: string | null): string {
  const head = Array.from(singleLine(channel)).slice(0, ALERT_TITLE_CHARS);
  const name = team ? Array.from(singleLine(team).trim()) : [];
  const room = ALERT_TITLE_CHARS - head.length - ' · '.length;
  if (!name.length || room < 2) return head.join('');
  const cut = name.length > room ? [...name.slice(0, room - 1), '…'] : name;
  return `${head.join('')} · ${cut.join('')}`;
}
