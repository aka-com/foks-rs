import type { ClipboardEvent } from 'react';
import { collapseWhitespace } from './name-normalization';

/** Preserve token boundaries before password inputs remove pasted newlines.
 * Only use for tokenized recovery/pairing phrases, never passphrases or PINs. */
export function pastePhrase(
  event: ClipboardEvent<HTMLInputElement>,
  onChange: (value: string) => void,
): void {
  event.preventDefault();
  const input = event.currentTarget;
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? start;
  // Keep boundary whitespace when pasting into part of an existing phrase.
  const pasted = event.clipboardData
    .getData('text/plain')
    .replace(/\p{White_Space}+/gu, ' ');
  onChange(
    collapseWhitespace(
      input.value.slice(0, start) + pasted + input.value.slice(end),
    ),
  );
}
