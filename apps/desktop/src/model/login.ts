/**
 * The login layout the New password sheet writes, read the way the agent and
 * the native host read it.
 *
 * A login is one small file of `key: value` lines (`user: …`, `password: …`,
 * `url: …`). The agent's listing reports `shape: 'login'` for such a file and
 * `copy_item_field` copies one field, both through `foks_client::kv::login`.
 * This module applies the same rules to text the renderer already holds after
 * Show or Edit, so the fields it displays are the fields Copy finds:
 *
 * - Lines are separated by `\n`, and one trailing `\r` is removed from each.
 * - A field line splits at its first `:`. The key is not trimmed; it must be
 *   non-empty and contain no ASCII whitespace (space, tab, line feed, form
 *   feed, carriage return), so a sentence such as `Reset it at https://x` is
 *   not a field line. A line whose `:` is followed directly by `//`, such as
 *   a bare `https://example.com`, is a URL and not a field line either.
 * - The value loses exactly one leading space, the separator the sheet
 *   writes; any other whitespace is part of the value.
 * - Keys match exactly, with the aliases `user`/`username` and
 *   `url`/`website`; the first line naming a field wins, and an empty value
 *   is absent.
 * - A text is a login when every line that is not blank is a field line and
 *   one has the key `password`.
 */

/** A login field `copy_item_field` can copy. */
export type LoginFieldName = 'username' | 'password' | 'url';

/** What the agent's listing reports for a small file. */
export type ItemShape = 'login' | 'other';

/** The order login fields are listed in. */
export const LOGIN_FIELDS: readonly LoginFieldName[] = [
  'username',
  'password',
  'url',
];

const ALIASES: Readonly<Record<LoginFieldName, readonly string[]>> = {
  username: ['user', 'username'],
  password: ['password'],
  url: ['url', 'website'],
};

/**
 * One line of a login: the key and value of a field line, or `key: null` and
 * the line itself for any other line. `separator` is the space that followed
 * the `:`, or `''` when there was none or the line is not a field line. `raw`
 * is the line as stored, with its `\r`, so an editor can write back a line it
 * did not change.
 */
export interface LoginLine {
  key: string | null;
  value: string;
  separator: '' | ' ';
  raw: string;
}

const SPACE = /[\t\n\f\r ]/;
const BLANK = /^[\t\n\f\r ]*$/;

/** Every line of a login's text, in order. */
export function loginLines(text: string): LoginLine[] {
  return text.split('\n').map((raw) => {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    const cut = line.indexOf(':');
    const key = cut < 0 ? '' : line.slice(0, cut);
    const rest = line.slice(cut + 1);
    if (!key || SPACE.test(key) || rest.startsWith('//'))
      return { key: null, value: line, separator: '', raw };
    return rest.startsWith(' ')
      ? { key, value: rest.slice(1), separator: ' ', raw }
      : { key, value: rest, separator: '', raw };
  });
}

/**
 * The stored form of a field line with a new value, keeping the line's own
 * separator. A space is added after a bare `:` only when the value begins
 * with a space or `//`, which would otherwise be read back differently.
 */
export function fieldLineText(
  key: string,
  separator: '' | ' ',
  value: string,
): string {
  const spaced =
    separator === ' ' || value.startsWith(' ') || value.startsWith('//');
  return `${key}:${spaced ? ' ' : ''}${value}`;
}

/** The login field a key names, if any. */
export function loginFieldOf(key: string | null): LoginFieldName | null {
  if (key === null) return null;
  return LOGIN_FIELDS.find((field) => ALIASES[field].includes(key)) ?? null;
}

/** The value of the first line naming `field`, or null when absent or empty. */
export function loginField(text: string, field: LoginFieldName): string | null {
  const line = loginLines(text).find(({ key }) => loginFieldOf(key) === field);
  return line?.value ? line.value : null;
}

/** Whether a line is blank: not a field line, and only ASCII whitespace. */
export function isBlankLine({ key, raw }: LoginLine): boolean {
  return key === null && BLANK.test(raw);
}

/** The shape the agent's listing reports for a small file with this text. */
export function shapeOf(text: string): ItemShape {
  const lines = loginLines(text);
  return lines.every((line) => line.key !== null || isBlankLine(line)) &&
    lines.some(({ key }) => key === 'password')
    ? 'login'
    : 'other';
}
