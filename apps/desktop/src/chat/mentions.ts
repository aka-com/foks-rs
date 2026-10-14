import { admits, parseRole } from '../model';
import type { AgentSnapshot, RoleWire } from '../model';

/**
 * Which `@name` tokens in message text are drawn as mentions. This is text
 * styling of Basic messages only: a mention has no protocol meaning and is
 * not counted. It raises a desktop alert only in a channel whose alert mode
 * is Mentions only, through {@link mentionsReader}.
 */
export interface ChatMentions {
  /** The team members' usernames, as {@link mentionKey} forms. */
  members: ReadonlySet<string>;
  /** The reader's own username as a {@link mentionKey} form, if known. */
  self: string | null;
}

/**
 * Letters that FOKS v0.1.9 username normalization replaces before it removes
 * combining marks (`flatten_unicode` in `crates/foks-verify/src/user.rs`).
 * Without this, `ł`, `ø`, `æ` and `ß` keep their non-ASCII form.
 */
const FLATTEN: Readonly<Record<string, string>> = {
  ą: 'a',
  ć: 'c',
  ę: 'e',
  ł: 'l',
  ń: 'n',
  ó: 'o',
  ś: 's',
  ż: 'z',
  ź: 'z',
  ø: 'o',
  æ: 'a',
  ß: 's',
};

/**
 * The form a username is compared in, following FOKS v0.1.9 username
 * normalization: lower case, the {@link FLATTEN} letters replaced, combining
 * marks removed and `.` and `-` read as `_`. So `@Priya.N` and `@priya_n`
 * both name `priya.n`, and `@lukasz` names `Łukasz`. The validity checks
 * (ASCII only, length) are not applied; a name that fails them matches no
 * roster username.
 */
export function mentionKey(name: string): string {
  return name
    .toLowerCase()
    .replace(/[ąćęłńóśżźøæß]/gu, (c) => FLATTEN[c] ?? c)
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .replace(/[.-]/g, '_');
}

/** The mentions drawn for a team's usernames and the reader's own. */
export function chatMentions(
  usernames: Iterable<string>,
  self: string | null | undefined,
): ChatMentions {
  const members = new Set(Array.from(usernames, mentionKey));
  const own = self ? mentionKey(self) : null;
  if (own) members.add(own);
  return { members, self: own };
}

/**
 * An `@name` where the message renderer draws one: not continuing a word, a
 * name or an email address (`bob@example.com`).
 */
const MENTION =
  /(?<![\p{L}\p{M}\p{N}_@.])@([\p{L}\p{N}][\p{L}\p{M}\p{N}._-]*)/gu;

/**
 * Whether `text` names `username` as an `@name`, compared as
 * {@link mentionKey} compares names, with the trailing `.`, `_` or `-` that
 * ends a sentence left out as the renderer leaves it out. This is a match on
 * the text on this device, for the Mentions only alert mode: the pinned
 * protocol has no mentions, and a sender can write any name.
 */
export function mentionsReader(text: string, username: string): boolean {
  const self = mentionKey(username);
  for (const match of text.matchAll(MENTION)) {
    let end = match[1].length;
    while (end > 1 && '._-'.includes(match[1][end - 1])) end--;
    if (mentionKey(match[1].slice(0, end)) === self) return true;
  }
  return false;
}

/**
 * The reader's username in a team store: that of the account the store's
 * chat scope names (`accountAlias`) on the store's server; `null` when the
 * snapshot holds no such account.
 */
export function readerUsername(
  snapshot: AgentSnapshot,
  storeId: string,
  accountAlias: string,
): string | null {
  const store = snapshot.stores.find((candidate) => candidate.id === storeId);
  if (store?.kind !== 'team') return null;
  return (
    snapshot.accounts.find(
      (account) =>
        account.server === store.server && account.alias === accountAlias,
    )?.username ?? null
  );
}

/** A team member an `@name` can be completed to. */
export interface ChatMember {
  username: string;
  /** The member's role in the team, as the roster states it. */
  role: RoleWire;
}

/** The members a channel's `@name` completion offers. */
export interface ChannelRoster {
  members: readonly ChatMember[];
  /**
   * How many team members the channel's read role leaves out. A message
   * that names one of them is still sent, but they cannot read it.
   */
  excluded: number;
}

/**
 * A channel's read role as the chat contract writes it (`Owner`, `Admin`,
 * `Member (0)`), in the form `admits` compares; `null` for any other text.
 */
export function channelRole(text: string): RoleWire | null {
  const member = /^Member \((-?\d+)\)$/.exec(text);
  if (member) return { role: 'Member', visibility: Number(member[1]) };
  return text === 'Owner' || text === 'Admin' ? { role: text } : null;
}

/**
 * The members whose role admits `need`, the role a channel's reader needs
 * (`channelReadRole` in `audience.ts`). A `null` role leaves every member
 * in, as does a member whose own role this cannot read: the completion is a
 * convenience, and the server enforces who reads the channel.
 */
export function channelRoster(
  members: readonly ChatMember[],
  need: RoleWire | null,
): ChannelRoster {
  if (!need) return { members, excluded: 0 };
  const readers = members.filter(
    (member) => !parseRole(member.role) || admits(member.role, need),
  );
  return { members: readers, excluded: members.length - readers.length };
}

/** How many completions the list offers at once. */
export const MENTION_CHOICES = 8;

/**
 * The members whose username starts with `query`, or has a part after a
 * `.`, `_` or `-` that does, compared as {@link mentionKey} compares names.
 * Names that start with the query come first, then alphabetical order.
 */
export function matchMembers(
  members: readonly ChatMember[],
  query: string,
): ChatMember[] {
  const key = mentionKey(query);
  const ranked: { member: ChatMember; whole: boolean; name: string }[] = [];
  for (const member of members) {
    const name = mentionKey(member.username);
    const whole = name.startsWith(key);
    if (whole || name.split('_').some((part) => part.startsWith(key)))
      ranked.push({ member, whole, name });
  }
  ranked.sort(
    (a, b) => Number(b.whole) - Number(a.whole) || a.name.localeCompare(b.name),
  );
  return ranked.slice(0, MENTION_CHOICES).map(({ member }) => member);
}
