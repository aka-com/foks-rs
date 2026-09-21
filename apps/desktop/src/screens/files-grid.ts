import { virtualListWindow } from '../../kit/virtual-list';
import type {
  VirtualListInput,
  VirtualListWindow,
} from '../../kit/virtual-list';

/** Keep these dimensions aligned with the card rules in files.css. */
export const FILES_CARD_WIDTH = 160;
export const FILES_CARD_HEIGHT = 98;
export const FILES_CARD_HEIGHT_WITH_LOCATION = 120;
export const FILES_CARD_GAP = 10;

export interface FilesGridWindow extends VirtualListWindow {
  columns: number;
  /** Gap after the mounted cards when another row follows in the spacer. */
  trailingGap: number;
}

/** Window complete grid rows, then translate the row range to item indices. */
export function filesGridWindow({
  count,
  width,
  location = false,
  ...viewport
}: Omit<VirtualListInput, 'heights'> & {
  count: number;
  width: number;
  location?: boolean;
}): FilesGridWindow {
  const columns = Math.max(
    1,
    Math.floor(
      (Math.max(0, width) + FILES_CARD_GAP) /
        (FILES_CARD_WIDTH + FILES_CARD_GAP),
    ),
  );
  const rows = Math.ceil(count / columns);
  const cardHeight = location
    ? FILES_CARD_HEIGHT_WITH_LOCATION
    : FILES_CARD_HEIGHT;
  const window = virtualListWindow({
    ...viewport,
    heights: Array.from({ length: rows }, (_, row) =>
      row === rows - 1 ? cardHeight : cardHeight + FILES_CARD_GAP,
    ),
  });
  return {
    ...window,
    columns,
    start: window.start * columns,
    end: Math.min(count, window.end * columns),
    trailingGap: window.end < rows ? FILES_CARD_GAP : 0,
  };
}
