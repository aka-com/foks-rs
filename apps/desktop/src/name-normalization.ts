/** Mirrors foks-verify's v0.1.9 username normalization. Do not use NFKD:
 * compatibility characters are not accepted by the protocol. */
export function normalizeUsername(input: string): string | null {
  const special: Record<string, string> = {
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
  const normalized = [...input.toLowerCase()]
    .map((c) => special[c] ?? c)
    .join('')
    .normalize('NFD')
    .replace(/\p{Mark}/gu, '')
    .replace(/[.-]/gu, '_');
  if (
    normalized.length < 3 ||
    normalized.length > 25 ||
    /[^a-z0-9_]/u.test(normalized) ||
    normalized.startsWith('_') ||
    normalized.endsWith('_') ||
    normalized.includes('__')
  )
    return null;
  return normalized;
}

/** Rust split_whitespace uses Unicode White_Space, unlike JavaScript \s. */
export function collapseWhitespace(input: string, separator = ' '): string {
  return input
    .split(/\p{White_Space}+/u)
    .filter(Boolean)
    .join(separator);
}

/**
 * Returns the server team name, or null if the name is invalid. This mirrors
 * `server_team_name` in foks-client-app: whitespace is collapsed to an
 * underscore, then the lowercase result must contain 3 to 25 letters, digits,
 * or single underscores after dots and dashes are replaced with underscores.
 */
export function serverTeamName(name: string): string | null {
  const folded = collapseWhitespace(name, '_');
  return normalizeUsername(folded) === null ? null : folded;
}

/** Converts a display name to its local team alias. */
export function teamAliasOf(name: string): string {
  return (normalizeUsername(collapseWhitespace(name, '_')) ?? name)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}
