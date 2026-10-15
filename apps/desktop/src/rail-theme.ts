/**
 * The rail's persisted color preference.
 *
 * Persisted in `localStorage` as `railColor` and applied as `data-rail` on the
 * document. Rules in `kit/tokens.css` map it to `--rail` for dependent
 * components. Unrecognized values use the default `--rail` value.
 */

import {
  readStorage,
  STORAGE_KEYS,
  writeStorage,
} from './platform/device-storage';

export interface RailColor {
  id: string;
  label: string;
  /** The paired colors from `tokens.css`, used by the split preview. */
  hex: string;
  darkHex: string;
}

export const RAIL_COLORS: readonly RailColor[] = [
  { id: 'default', label: 'Blue', hex: '#4c6be1', darkHex: '#3c4e91' },
  { id: 'iris', label: 'Iris', hex: '#6a6ff0', darkHex: '#5a4f96' },
  { id: 'sky', label: 'Sky', hex: '#3b81f6', darkHex: '#315d99' },
  { id: 'slate', label: 'Slate', hex: '#5468b3', darkHex: '#46578e' },
];

export const DEFAULT_RAIL_COLOR = RAIL_COLORS[0].id;

const KEY = STORAGE_KEYS.railColor;

function known(id: string | null | undefined): string {
  if (id === 'lavender') return 'iris';
  return RAIL_COLORS.some((color) => color.id === id)
    ? (id as string)
    : DEFAULT_RAIL_COLOR;
}

/** The stored color id; the default when unset or unreadable. */
export function storedRailColor(): string {
  return known(readStorage(KEY));
}

/** Persists the color id. */
export function rememberRailColor(id: string): void {
  // Storage may be unavailable; the preference then lasts for this session only.
  writeStorage(KEY, id);
}

/** Draws the color: `data-rail` on the document, which the tokens read. */
export function applyRailColor(id: string): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  const resolved = known(id);
  if (resolved === DEFAULT_RAIL_COLOR) delete root.dataset.rail;
  else root.dataset.rail = resolved;
}
