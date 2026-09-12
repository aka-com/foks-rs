import type { ChatMessage } from '../chat-contract';
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
export function notificationText(
  messages: readonly ChatMessage[],
  incomplete: boolean,
): { body?: string; count: number; incomplete: boolean }[] {
  const first = messages.slice(0, 3).map((m) => ({
    body:
      m.content.kind === 'text'
        ? Array.from(m.content.text).slice(0, 256).join('')
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
