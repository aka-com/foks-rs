import type { ChatChannel } from '../chat-contract';
export function channelTitle(channel: ChatChannel): string {
  return `# ${channel.name || 'general'}`;
}

export function accessSummary(channel: ChatChannel): string {
  if (!channel.readable) return 'Read access required';
  if (channel.read_role === channel.write_role)
    return `${channel.read_role} can read and write`;
  return `Read ${channel.read_role} · Write ${channel.write_role}`;
}

export function messageDate(milliseconds: string): Date | null {
  const value = BigInt(milliseconds);
  if (value > 8_640_000_000_000_000n) return null;
  const date = new Date(Number(value));
  return Number.isNaN(date.valueOf()) ? null : date;
}

export function messageTime(milliseconds: string): string {
  const date = messageDate(milliseconds);
  if (!date) return `Time ${milliseconds}`;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}
