/**
 * Display formatting — ported from `wave6/shell.js:163-173`.
 *
 * Pure string arithmetic with no view attached, so the same numbers appear in
 * a row, a chip and a test.
 */

/**
 * Bytes as the design writes them: bytes under 1 kB, one decimal of kB up to
 * 10 kB and whole kB above it, one decimal of MB from 1 MB.
 */
export function fmtSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1e6) return `${(bytes / 1000).toFixed(bytes < 10000 ? 1 : 0)} KB`;
  return `${(bytes / 1e6).toFixed(1)} MB`;
}

/**
 * Up to two initials from a display name.
 *
 * The mail domain is dropped first, then the name is split on `.`, `-`, `_`
 * and spaces: `sam.ortiz` → `SO`, `deploy-bot` → `DB`, `rae` → `R`.
 */
export function initials(name: string): string {
  return name
    .replace(/@.*/, '')
    .split(/[.\-_ ]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0].toUpperCase())
    .join('');
}

/** The avatar palette, in the order `shell.js` declares it. */
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
 * A stable avatar colour for a name.
 *
 * Sum of code points modulo the palette — deterministic, so the same person
 * is the same colour on every screen and in every screenshot.
 */
export function hue(name: string): string {
  const sum = [...name].reduce((total, char) => total + char.charCodeAt(0), 0);
  return HUES[sum % HUES.length];
}
