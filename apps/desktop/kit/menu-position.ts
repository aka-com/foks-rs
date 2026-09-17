export interface MenuRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface MenuSize {
  width: number;
  height: number;
}

export interface ViewportSize {
  width: number;
  height: number;
}

export type MenuAlign = 'start' | 'end' | 'center';

/**
 * Anchor a menu to its trigger and keep every edge inside the viewport.
 * Prefer opening below, flip above when that is the only side that fits, and
 * clamp as a final fallback for menus taller than either available side.
 *
 * `align` controls the horizontal origin against the trigger:
 * - `end` (default): right edges match (connection ⋯, Copy ▾)
 * - `start`: left edges match (sheet reconnect ⋯)
 * - `center`: menu centered on the trigger (Connect-agents blanks)
 */
export function anchoredMenuPosition(
  anchor: MenuRect,
  menu: MenuSize,
  viewport: ViewportSize,
  gap = 4,
  inset = 8,
  align: MenuAlign = 'end',
): { left: number; top: number } {
  const maxLeft = Math.max(inset, viewport.width - menu.width - inset);
  const preferredLeft =
    align === 'start'
      ? anchor.left
      : align === 'center'
        ? anchor.left + (anchor.right - anchor.left) / 2 - menu.width / 2
        : anchor.right - menu.width;
  const left = Math.min(Math.max(inset, preferredLeft), maxLeft);

  const below = anchor.bottom + gap;
  const above = anchor.top - menu.height - gap;
  const fitsBelow = below + menu.height <= viewport.height - inset;
  const fitsAbove = above >= inset;
  const preferredTop = fitsBelow || !fitsAbove ? below : above;
  const maxTop = Math.max(inset, viewport.height - menu.height - inset);
  const top = Math.min(Math.max(inset, preferredTop), maxTop);

  return { left, top };
}

/** Apply `anchoredMenuPosition` to a fixed portal wrap and reveal it. */
export function placeAnchoredMenu(
  wrap: HTMLElement,
  trigger: HTMLElement,
  align: MenuAlign = 'end',
  gap = 4,
): void {
  const position = anchoredMenuPosition(
    trigger.getBoundingClientRect(),
    wrap.getBoundingClientRect(),
    { width: window.innerWidth, height: window.innerHeight },
    gap,
    8,
    align,
  );
  wrap.style.left = `${position.left}px`;
  wrap.style.top = `${position.top}px`;
  wrap.style.visibility = 'visible';
}
