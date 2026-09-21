/**
 * Text and numeric display formatting helpers.
 */

/**
 * Formats a byte count into human-readable units (B, KB, MB).
 */
export function fmtSize(bytes: number | null): string {
  if (bytes === null) return 'Unknown';
  if (bytes === 0) return '0 B';
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1e6) return `${(bytes / 1000).toFixed(bytes < 10000 ? 1 : 0)} KB`;
  return `${(bytes / 1e6).toFixed(1)} MB`;
}

/**
 * Up to two initials from a display name.
 *
 * The mail domain is dropped first, then the name is split on `.`, `-`, `_`
 * and spaces: `sam.ortiz` → `SO`, `deploy-bot` → `DB`, `satoshi` → `S`.
 */
export function initials(name: string): string {
  // Words are runs of letters and digits: punctuation such as the "(" in
  // "Work (Acme)" never becomes an initial.
  return name
    .replace(/@.*/, '')
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0].toUpperCase())
    .join('');
}

/** Palette of background colors used for user avatars. */
export const HUES = [
  '#5e5ce6',
  '#ff9f0a',
  '#30b0c7',
  '#ff2d55',
  '#34c759',
  '#af52de',
  '#0a7cff',
  '#a2845e',
] as const;

/**
 * A stable avatar color for a name.
 *
 * Returns a deterministic avatar color based on the sum of character codes in the name.
 */
export function hue(name: string): string {
  const sum = [...name].reduce((total, char) => total + char.charCodeAt(0), 0);
  return HUES[sum % HUES.length];
}

/**
 * Truncates a hex identifier with an ellipsis, preserving leading characters
 * and the specified number of trailing characters.
 */
export function shortId(value: string, tail = 4): string {
  return value.length > 10 + tail
    ? `${value.slice(0, 10)}…${value.slice(-tail)}`
    : value;
}

/** "1 group" / "0 groups" — the count and the word that agrees with it. */
export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}
