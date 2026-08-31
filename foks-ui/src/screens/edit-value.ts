import { kindOf } from '../model';
import type { Item } from '../model';

/**
 * The mock keeps a harmless structured login skeleton in catalog fixture
 * data, while its exact read returns only the password. Native catalog rows
 * carry no value at all and the exact read returns the complete text. Keep
 * those two test/production shapes equivalent before an edit is shown.
 */
export function editableValue(item: Item, learned: string): string {
  if (kindOf(item) !== 'Password' || !item.value) return learned;
  if (learned.split('\n').some((line) => line.startsWith('password: '))) {
    return learned;
  }
  return item.value.replace(/password: .*/, `password: ${learned}`);
}
