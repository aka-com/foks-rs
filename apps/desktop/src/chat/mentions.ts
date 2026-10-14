/**
 * Which `@name` tokens in message text are drawn as mentions. This is text
 * styling of Basic messages only: a mention has no protocol meaning, raises
 * no notification and is not counted.
 */
export interface ChatMentions {
  /** The team members' usernames, as {@link mentionKey} forms. */
  members: ReadonlySet<string>;
  /** The reader's own username as a {@link mentionKey} form, if known. */
  self: string | null;
}

/**
 * The form a username is compared in: lower case, with combining marks
 * removed and `.` and `-` read as `_`, as FOKS username normalization treats
 * them, so `@Priya.N` and `@priya_n` both name `priya.n`.
 */
export function mentionKey(name: string): string {
  return name
    .toLowerCase()
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
