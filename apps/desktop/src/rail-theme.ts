/**
 * The rail's persisted colour preference.
 *
 * Persisted in `localStorage` as `railColor` and applied as `data-rail` on the
 * document. Rules in `kit/tokens.css` map it to `--rail` for dependent
 * components. Unrecognized values use the default `--rail` value.
 */

export interface RailColor {
  id: string;
  label: string;
  /** The hex `tokens.css` sets for this id; here so a picker can draw it. */
  hex: string;
}

export const RAIL_COLORS: readonly RailColor[] = [
  { id: 'default', label: 'Blue', hex: '#4c6be1' },
  { id: 'iris', label: 'Iris', hex: '#6a6ff0' },
  { id: 'lavender', label: 'Lavender', hex: '#7a77f1' },
  { id: 'sky', label: 'Sky', hex: '#4f8ef7' },
  { id: 'slate', label: 'Slate', hex: '#5468b3' },
];

export const DEFAULT_RAIL_COLOR = RAIL_COLORS[0].id;

const KEY = 'railColor';

function known(id: string | null | undefined): string {
  return RAIL_COLORS.some((color) => color.id === id)
    ? (id as string)
    : DEFAULT_RAIL_COLOR;
}

/** The stored colour id; the default when unset or unreadable. */
export function storedRailColor(): string {
  try {
    return known(
      typeof window !== 'undefined' ? window.localStorage.getItem(KEY) : null,
    );
  } catch {
    return DEFAULT_RAIL_COLOR;
  }
}

/** Persists the colour id. */
export function rememberRailColor(id: string): void {
  try {
    if (typeof window !== 'undefined') window.localStorage.setItem(KEY, id);
  } catch {
    // Storage is unavailable; the preference lasts for this session only.
  }
}

/** Draws the colour: `data-rail` on the document, which the tokens read. */
export function applyRailColor(id: string): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const resolved = known(id);
  if (resolved === DEFAULT_RAIL_COLOR) delete root.dataset.rail;
  else root.dataset.rail = resolved;
}
