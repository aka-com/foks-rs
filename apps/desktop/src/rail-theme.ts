/**
 * Persisted accent color preference.
 *
 * Stored in `localStorage` under `railColor` and applied as `data-rail` on the
 * document root (both names predate the redesign, when this setting tinted the
 * sidebar). `kit/tokens.css` maps `data-rail` to `--accent-hue` and
 * `--accent-fill` for buttons, focus rings, links, and selection washes. The
 * sidebar remains neutral. Unrecognized values fall back to the default accent.
 */

export interface RailColor {
  id: string;
  label: string;
  /** `--accent-fill` value from `tokens.css` rendered in the swatch (shared across light and dark themes). */
  fill: string;
}

/** Distinct accent hues that avoid the semantic danger red and warning amber colors. */
export const RAIL_COLORS: readonly RailColor[] = [
  { id: 'default', label: 'Blue', fill: '#4c6be1' },
  { id: 'purple', label: 'Purple', fill: '#8048d9' },
  { id: 'pink', label: 'Pink', fill: '#c43a7e' },
  { id: 'green', label: 'Green', fill: '#22803f' },
  { id: 'graphite', label: 'Graphite', fill: '#5f646d' },
];

/** Maps legacy accent color IDs to their current equivalents. */
const RETIRED: Readonly<Record<string, string>> = {
  lavender: 'purple',
  iris: 'purple',
  sky: 'default',
  slate: 'graphite',
};

export const DEFAULT_RAIL_COLOR = RAIL_COLORS[0].id;

const KEY = 'railColor';

/** Resolves a stored color ID to a valid current ID, mapping legacy IDs and
 *  falling back to the default for unknown values. */
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
