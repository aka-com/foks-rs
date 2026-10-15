/**
 * Copy actions for vault items: which copies an item offers, the command
 * each one calls, and the toast that names what reached the clipboard. Open
 * website, which the host also performs without returning the value, is
 * here for the same reason.
 */

import type { ToastController } from '../../kit/toasts';
import { kindOf } from '../model';
import type { Item } from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge, ClipboardCopyResponse, ItemField } from '../bridge';
import { itemRefOf, rememberRecent } from '../recent-favorites';

/** A copy an item offers, labelled for its button and menu item. */
export interface CopyAction {
  field: ItemField;
  label: string;
}

/** The label of each field's Copy button and menu item. */
export const COPY_LABELS: Readonly<Record<ItemField, string>> = {
  password: 'Copy password',
  username: 'Copy user name',
  url: 'Copy website',
  value: 'Copy value',
};

const copyOf = (field: ItemField): CopyAction => ({
  field,
  label: COPY_LABELS[field],
});

const PASSWORD_COPIES: readonly CopyAction[] = [
  copyOf('password'),
  copyOf('username'),
];
const VALUE_COPIES: readonly CopyAction[] = [copyOf('value')];
// A Password known only by its `/logins/` path, from an agent that lists no
// shape, may not hold `password:` lines, so it also offers its whole text.
const UNSHAPED_PASSWORD_COPIES: readonly CopyAction[] = [
  ...PASSWORD_COPIES,
  ...VALUE_COPIES,
];

/**
 * The copies a row offers: a Password's password and user name, a text
 * Document's whole value, and none for a file, which is downloaded instead.
 * A Password whose listing has no shape also offers its whole value.
 */
export function copyActions(item: Item): readonly CopyAction[] {
  if (item.kind === 'File') return [];
  if (kindOf(item) !== 'Password') return VALUE_COPIES;
  return item.shape === undefined ? UNSHAPED_PASSWORD_COPIES : PASSWORD_COPIES;
}

/**
 * Whether the item offers Open website: a Password, whose `url` line the
 * host reads. Whether that line holds an http or https address is known only
 * when the host reads it.
 */
export function opensWebsite(item: Item): boolean {
  return item.kind !== 'File' && kindOf(item) === 'Password';
}

/** The toast after a copy, naming only what was copied. */
export const COPIED: Readonly<Record<ItemField, string>> = {
  password: 'Password copied',
  username: 'User name copied',
  url: 'Website copied',
  value: 'Value copied',
};

/**
 * The toast after a copy: what was copied and when the host clears the
 * clipboard, for example "Password copied · clears in 30 s".
 */
export function copiedMessage(
  field: ItemField,
  response: ClipboardCopyResponse,
): string {
  return `${COPIED[field]} · clears in ${response.clearAfterSeconds} s`;
}

/**
 * Copies one field of the item's current version. A login field is parsed
 * out by the host, so only that field reaches the clipboard; `value` copies
 * the whole stored text. A completed copy puts the item at the head of the
 * ⌘K palette's Recent list.
 */
export async function copyItem(
  bridge: Bridge,
  item: Item,
  field: ItemField,
): Promise<ClipboardCopyResponse> {
  const request = {
    storeId: item.store,
    path: item.path,
    version: item.version,
  };
  const response = await (field === 'value'
    ? bridge.copyItemValue(request)
    : bridge.copyItemField({ ...request, field }));
  rememberRecent(itemRefOf(item));
  return response;
}

/** The message for a copy refused because the item lacks the field. */
export function missingFieldMessage(error: unknown): string | null {
  const typed = normalizeCommandError(error);
  return typed.code === 'field-missing' ? typed.message : null;
}

/** One copy action and where its outcome is reported. */
export interface AnnouncedCopy {
  bridge: Bridge;
  item: Item;
  field: ItemField;
  toasts: Pick<ToastController, 'show'>;
  /** Why the caller refuses the copy, shown instead of copying. */
  problem?: string;
  /** Any failure other than a missing field. */
  onError: (error: unknown, item: Item) => void;
}

/**
 * A row's, the details panel's or the palette's Copy: refuses with the
 * access `problem` as a warning when there is one, copies the field, and
 * names in a toast what was copied and when the clipboard is cleared, or
 * which field the item lacks. The copy toast closes by the time the host
 * clears the clipboard, even while it is hovered or focused.
 */
export async function copyAndAnnounce({
  bridge,
  item,
  field,
  toasts,
  problem,
  onError,
}: AnnouncedCopy): Promise<void> {
  if (problem) {
    toasts.show(problem, { tone: 'warning' });
    return;
  }
  try {
    // Read before the copy, so the toast closes no later than the host's
    // clear, which a hold on the toast does not delay.
    const copiedAt = Date.now();
    const response = await copyItem(bridge, item, field);
    toasts.show(copiedMessage(field, response), {
      expiresAt: copiedAt + response.clearAfterSeconds * 1_000,
    });
  } catch (error) {
    const missing = missingFieldMessage(error);
    if (missing) toasts.show(missing, { tone: 'warning' });
    else onError(error, item);
  }
}

/** Open website, with the same checks and reports as {@link AnnouncedCopy}. */
export type AnnouncedOpen = Omit<AnnouncedCopy, 'field'>;

/**
 * The palette's Open website: refuses with the access `problem` as a
 * warning when there is one, asks the host to open the login's website in
 * the default browser, and puts the item at the head of Recent. An item
 * without a website, or whose website is not an http or https address, is
 * reported in a warning toast.
 */
export async function openWebsiteAndAnnounce({
  bridge,
  item,
  toasts,
  problem,
  onError,
}: AnnouncedOpen): Promise<void> {
  if (problem) {
    toasts.show(problem, { tone: 'warning' });
    return;
  }
  try {
    await bridge.openItemUrl({
      storeId: item.store,
      path: item.path,
      version: item.version,
    });
    rememberRecent(itemRefOf(item));
    toasts.show('Website opened in the browser');
  } catch (error) {
    const typed = normalizeCommandError(error);
    if (typed.code === 'field-missing' || typed.code === 'website-unsupported')
      toasts.show(typed.message, { tone: 'warning' });
    else onError(error, item);
  }
}
