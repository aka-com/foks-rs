/**
 * The details panel's logic that needs no React: the rows a login lists and
 * the lines its structured editor opens with, and the item read that
 * concurrent requests for one item version share and that must answer for
 * exactly the version asked for.
 */

import {
  isBlankLine,
  LOGIN_FIELDS,
  loginField,
  loginFieldOf,
  loginLines,
} from '../model';
import type { LoginFieldName } from '../model';
import type { Bridge, ItemRequest, ReadItemResponse } from '../bridge';

const FIELD_LABELS: ReadonlyMap<string, string> = new Map([
  ['user', 'User name'],
  ['username', 'User name'],
  ['password', 'Password'],
  ['url', 'Website'],
  ['website', 'Website'],
  ['ssid', 'Network'],
]);

/**
 * The label of a login line's key: a known field's name, or the key itself.
 * A map lookup, so a key such as `constructor` is not read from a prototype.
 */
export function fieldLabel(key: string): string {
  return FIELD_LABELS.get(key) ?? key;
}

/** Coalesces concurrent in-flight read requests for the same item. */
const readFlights = new WeakMap<
  Bridge,
  WeakMap<object, Map<string, Promise<ReadItemResponse>>>
>();

export function readOnce(
  bridge: Bridge,
  request: ItemRequest,
  accessGeneration: number,
  accessSession: object,
): Promise<ReadItemResponse> {
  const key = JSON.stringify([
    accessGeneration,
    request.storeId,
    request.path,
    request.version,
  ]);
  const sessions =
    readFlights.get(bridge) ??
    new WeakMap<object, Map<string, Promise<ReadItemResponse>>>();
  readFlights.set(bridge, sessions);
  const flights =
    sessions.get(accessSession) ?? new Map<string, Promise<ReadItemResponse>>();
  sessions.set(accessSession, flights);
  const current = flights.get(key);
  if (current) return current;
  const pending = bridge.readItem(request).finally(() => flights.delete(key));
  flights.set(key, pending);
  return pending;
}

export function assertExactRead(
  request: ItemRequest,
  response: ReadItemResponse,
): void {
  if (
    response.store !== request.storeId ||
    response.path !== request.path ||
    response.version !== request.version
  ) {
    throw new Error(
      'Received content for a different item or version than requested.',
    );
  }
}

/**
 * The lines the structured login editor lists, by index, or null when the
 * text opens in the textarea. The editor opens for text with a line the host
 * would copy as the password, which the editor masks, and lists every line
 * but blank ones: typing into a blank line would add a line that is not a
 * field line, and the agent would then list the item as a Document. Blank
 * lines are still saved as they were read. Both choices are made once, when
 * the edit opens, so typing never swaps editors or removes a row.
 */
export function loginEditLines(text: string): readonly number[] | null {
  const lines = loginLines(text);
  if (!lines.some(({ key }) => loginFieldOf(key) === 'password')) return null;
  return lines.flatMap((line, index) => (isBlankLine(line) ? [] : [index]));
}

/** One row of a login's Fields section. */
export interface LoginRow {
  /** Identifies the row's Show state: a login field, or `line:<index>`. */
  id: string;
  label: string;
  /** The field the host copies for this row; null for other lines. */
  field: LoginFieldName | null;
  /** Null before a read, and for a login field the text lacks. */
  value: string | null;
}

/**
 * The rows a login lists: user name, password and website always, each
 * masked until shown, then after a read any other line, such as a Wi-Fi
 * network name, in stored order. Values come from `loginField`, the parser
 * the host's field copy follows, so a row and its Copy agree.
 */
export function loginRows(shown: string | null): LoginRow[] {
  const rows: LoginRow[] = LOGIN_FIELDS.map((field) => ({
    id: field,
    label: fieldLabel(field),
    field,
    value: shown === null ? null : loginField(shown, field),
  }));
  if (shown === null) return rows;
  loginLines(shown).forEach(({ key, value }, index) => {
    if (loginFieldOf(key) !== null || !value) return;
    rows.push({
      id: `line:${index}`,
      label: key === null ? 'Text' : fieldLabel(key),
      field: null,
      value,
    });
  });
  return rows;
}
