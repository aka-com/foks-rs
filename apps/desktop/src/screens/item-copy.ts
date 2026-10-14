/**
 * Copy actions for vault items: which copies an item offers, the command
 * each one calls, and the toast that names what reached the clipboard.
 */

import { kindOf } from '../model';
import type { Item } from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge, CopyResponse, ItemField } from '../bridge';
import { itemRefOf, rememberRecent } from '../recent-favorites';

/** A copy an item offers, labelled for its button and menu item. */
export interface CopyAction {
  field: ItemField;
  label: string;
}

const PASSWORD_COPIES: readonly CopyAction[] = [
  { field: 'password', label: 'Copy password' },
  { field: 'username', label: 'Copy user name' },
];
const VALUE_COPIES: readonly CopyAction[] = [
  { field: 'value', label: 'Copy value' },
];

/**
 * The copies a row offers: a Password's password and user name, a text
 * Document's whole value, and none for a file, which is downloaded instead.
 */
export function copyActions(item: Item): readonly CopyAction[] {
  if (item.kind === 'File') return [];
  return kindOf(item) === 'Password' ? PASSWORD_COPIES : VALUE_COPIES;
}

/** The toast after a copy, naming only what was copied. */
export const COPIED: Readonly<Record<ItemField, string>> = {
  password: 'Password copied',
  username: 'User name copied',
  url: 'Website copied',
  value: 'Value copied',
};

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
): Promise<CopyResponse> {
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
