import { kindOf } from '../model';
import type { Item } from '../model';

// Default structured fields for new login items.
export function editableValue(item: Item, learned: string): string {
  if (kindOf(item) !== 'Password' || !item.value) return learned;
  if (learned.split('\n').some((line) => line.startsWith('password: '))) {
    return learned;
  }
  // Uses a replacer function so special replacement tokens remain literal.
  if (!/password: .*/.test(item.value)) return learned;
  return item.value.replace(/password: .*/, () => `password: ${learned}`);
}
