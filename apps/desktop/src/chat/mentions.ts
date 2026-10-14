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
