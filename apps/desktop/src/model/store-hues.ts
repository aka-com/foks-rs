/**
 * Tile colors for the sidebar's store marks.
 */

import { HUES, hue } from './format';
import type { Store, StoreRef } from './types';

/**
 * Assigns each store a tile color, keyed by `StoreRef`.
 *
 * Hashing the reference alone is not enough: with eight hues, two stores in a
 * short list collide often, and two identical tiles in the collapsed rail
 * defeat the point of coloring them. Each store takes its hashed hue when that
 * hue is still free; otherwise it takes the first unused palette entry, so only
 * the colliding store moves. Once the palette is exhausted, hashed hues repeat.
 *
 * The result depends only on the references and their order, so the same list
 * always produces the same colors.
 */
export function storeHues(stores: readonly Store[]): Map<StoreRef, string> {
  const colors = new Map<StoreRef, string>();
  const taken = new Set<string>();
  for (const store of stores) {
    if (colors.has(store.id)) continue;
    let color = hue(store.id);
    if (taken.has(color) && taken.size < HUES.length)
      color = HUES.find((candidate) => !taken.has(candidate)) ?? color;
    taken.add(color);
    colors.set(store.id, color);
  }
  return colors;
}
