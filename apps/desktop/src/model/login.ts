/**
 * The login layout the New password sheet writes, read the way the agent and
 * the native host read it.
 *
 * A login is one small file of `key: value` lines (`user: …`, `password: …`,
 * `url: …`). The agent's listing reports `shape: 'login'` for a small file
 * with a `password` key, and `copy_item_field` copies one field, both through
 * `foks_client::kv::login`. This module applies the same rules to text the
 * renderer already holds after Show or Edit, so the fields it displays are
 * the fields Copy finds: a line splits at its first `:`, key and value are
 * trimmed, keys match exactly with the aliases `user`/`username` and
 * `url`/`website`, the first line naming a field wins, and an empty value is
 * absent.
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

/** One line of a login: a trimmed key and value, or a line with no key. */
export interface LoginLine {
  key: string | null;
  value: string;
}

/** Every line of a login's text, in order. */
export function loginLines(text: string): LoginLine[] {
  return text.split('\n').map((line) => {
    const cut = line.indexOf(':');
    const key = cut < 0 ? '' : line.slice(0, cut).trim();
    return key
      ? { key, value: line.slice(cut + 1).trim() }
      : { key: null, value: line.trim() };
  });
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

/** The shape the agent's listing reports for a small file with this text. */
export function shapeOf(text: string): ItemShape {
  return loginLines(text).some(({ key }) => key === 'password')
    ? 'login'
    : 'other';
}
