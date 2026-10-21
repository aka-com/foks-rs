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

export const RAIL_COLORS: readonly RailColor[] = [
  { id: 'default', label: 'Blue', fill: '#4c6be1' },
  { id: 'iris', label: 'Iris', fill: '#5b62e6' },
  { id: 'sky', label: 'Sky', fill: '#2b6fe0' },
  { id: 'slate', label: 'Slate', fill: '#5468b3' },
];

export const DEFAULT_RAIL_COLOR = RAIL_COLORS[0].id;

const KEY = 'railColor';

function known(id: string | null | undefined): string {
  if (id === 'lavender') return 'iris';
  return RAIL_COLORS.some((color) => color.id === id)
    ? (id as string)
    : DEFAULT_RAIL_COLOR;
}

/** The stored color id; the default when unset or unreadable. */
export function storedRailColor(): string {
  try {
    return known(
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
  const resolved = known(id);
  if (resolved === DEFAULT_RAIL_COLOR) delete root.dataset.rail;
  else root.dataset.rail = resolved;
}
