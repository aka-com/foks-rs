/**
 * The persisted accent color preference.
 *
 * Persisted in `localStorage` as `railColor` and applied as `data-rail` on the
 * document; both names predate the redesign, when the choice tinted the
 * sidebar. Rules in `kit/tokens.css` map it to `--accent-hue` and
 * `--accent-fill` (buttons, focus rings, links, selection washes). The
 * sidebar itself is neutral. Unrecognized values use the default accent.
 */

export interface RailColor {
  id: string;
  label: string;
  /** The `--accent-fill` value from `tokens.css`, drawn by the swatch. The
   *  fill is the same in both appearances. */
  fill: string;
}

/** Hues far enough apart to tell at swatch size; none is the danger red or
 *  the warning amber, which carry meaning elsewhere. */
export const RAIL_COLORS: readonly RailColor[] = [
  { id: 'default', label: 'Blue', fill: '#4c6be1' },
  { id: 'purple', label: 'Purple', fill: '#8048d9' },
  { id: 'pink', label: 'Pink', fill: '#c43a7e' },
  { id: 'green', label: 'Green', fill: '#22803f' },
  { id: 'graphite', label: 'Graphite', fill: '#5f646d' },
];

/** Choices that were removed, and the one each stored value now means. */
const RETIRED: Readonly<Record<string, string>> = {
  lavender: 'purple',
  iris: 'purple',
  sky: 'default',
  slate: 'graphite',
};

export const DEFAULT_RAIL_COLOR = RAIL_COLORS[0].id;

const KEY = 'railColor';

/** A stored id as a current choice: retired ids map forward, unknown ones
 *  fall back to the default. */
export function resolveRailColor(id: string | null | undefined): string {
  if (id && RETIRED[id]) return RETIRED[id];
  return RAIL_COLORS.some((color) => color.id === id)
    ? (id as string)
    : DEFAULT_RAIL_COLOR;
}

/** The stored color id; the default when unset or unreadable. */
export function storedRailColor(): string {
  try {
    return resolveRailColor(
      typeof window !== 'undefined' ? window.localStorage.getItem(KEY) : null,
    );
  } catch {
    return DEFAULT_RAIL_COLOR;
  }
}

/** Persists the color id. */
export function rememberRailColor(id: string): void {
  try {
    if (typeof window !== 'undefined') window.localStorage.setItem(KEY, id);
  } catch {
    // Storage is unavailable; the preference lasts for this session only.
  }
}

/** Draws the color: `data-rail` on the document, which the tokens read. */
export function applyRailColor(id: string): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const resolved = resolveRailColor(id);
  if (resolved === DEFAULT_RAIL_COLOR) delete root.dataset.rail;
  else root.dataset.rail = resolved;
}
