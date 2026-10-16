/**
 * Typing a HESP recovery phrase back in: the token grammar, per-field checks,
 * word completion and the three positions setup asks for.
 *
 * A recovery phrase is 17 tokens that alternate word and number, starting and
 * ending with a word: nine BIP-39 words and eight numbers from 0 to 8191. HESP
 * has no checksum, so these checks catch only a token that cannot appear in
 * its position. A wrong word that is still in the dictionary passes here and
 * is found by the agent, which derives the key and compares it with the
 * account's backup keys.
 */

import { BIP39_ENGLISH } from '../../bip39-english';

export const RECOVERY_PHRASE_TOKENS = 17;
export const RECOVERY_NUMBER_LIMIT = 8192;

export type TokenKind = 'word' | 'number';

/** Zero-based positions 0, 2, … 16 hold words; 1, 3, … 15 hold numbers. */
export function tokenKind(index: number): TokenKind {
  return index % 2 === 0 ? 'word' : 'number';
}

/** "Word 4" or "Number 6": the token's ordinal within its kind. */
export function tokenLabel(index: number): string {
  return tokenKind(index) === 'word'
    ? `Word ${index / 2 + 1}`
    : `Number ${(index + 1) / 2}`;
}

const WORDS = new Set(BIP39_ENGLISH);

/** The index of the first word that is not before `prefix`. */
function lowerBound(prefix: string): number {
  let low = 0;
  let high = BIP39_ENGLISH.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if ((BIP39_ENGLISH[middle] ?? '') < prefix) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** Up to `limit` dictionary words that start with `prefix`, in order. */
export function completions(prefix: string, limit = 6): string[] {
  const value = prefix.trim().toLowerCase();
  if (!value || /[^a-z]/.test(value)) return [];
  const result: string[] = [];
  for (let index = lowerBound(value); index < BIP39_ENGLISH.length; index++) {
    const word = BIP39_ENGLISH[index] ?? '';
    if (!word.startsWith(value) || result.length >= limit) break;
    result.push(word);
  }
  return result;
}

/** How a field stores what is typed: no spaces, and words in lower case. */
export function normalizeToken(index: number, value: string): string {
  const compact = value.replace(/\s+/g, '');
  return tokenKind(index) === 'word' ? compact.toLowerCase() : compact;
}

export type TokenProblem =
  | 'expected-word'
  | 'expected-number'
  | 'unknown-word'
  | 'unfinished-word'
  | 'number-range';

/**
 * What is wrong with one field, or null when it is empty or acceptable so far.
 * `final` is set once the field has been left: a prefix of a dictionary word is
 * then unfinished rather than still being typed.
 */
export function tokenProblem(
  index: number,
  value: string,
  final: boolean,
): TokenProblem | null {
  if (!value) return null;
  if (tokenKind(index) === 'word') {
    if (/^-?\d+$/.test(value)) return 'expected-word';
    if (WORDS.has(value)) return null;
    if (/[^a-z]/.test(value) || completions(value, 1).length === 0)
      return 'unknown-word';
    return final ? 'unfinished-word' : null;
  }
  if (/^-\d+$/.test(value)) return 'number-range';
  if (/\D/.test(value)) return 'expected-number';
  return Number(value) >= RECOVERY_NUMBER_LIMIT ? 'number-range' : null;
}

export const TOKEN_PROBLEM_TEXT: Readonly<Record<TokenProblem, string>> = {
  'expected-word': 'Expected a word',
  'expected-number': 'Expected a number',
  'unknown-word': 'Unknown word',
  'unfinished-word': 'Unfinished word',
  'number-range': 'Numbers go from 0 to 8191',
};

/** Whether a field holds a complete token for its position. */
export function tokenComplete(index: number, value: string): boolean {
  return value !== '' && tokenProblem(index, value, true) === null;
}

/**
 * Spreads pasted text over the fields from `start`, one token per field, so a
 * whole phrase pasted into the first field fills all 17. Tokens past the last
 * field are dropped.
 */
export function spreadTokens(
  tokens: readonly string[],
  start: number,
  pasted: string,
): string[] {
  const next = [...tokens];
  const parts = pasted.split(/\s+/).filter(Boolean);
  parts.forEach((part, offset) => {
    const index = start + offset;
    if (index < RECOVERY_PHRASE_TOKENS)
      next[index] = normalizeToken(index, part);
  });
  return next;
}

/** The phrase the agent is sent: the fields joined by single spaces. */
export function joinTokens(tokens: readonly string[]): string {
  return tokens.join(' ');
}

/**
 * Three distinct zero-based positions for setup's recall check, in order,
 * with at least one word and one number. `random` returns values in [0, 1).
 */
export function recallPositions(random: () => number = Math.random): number[] {
  for (;;) {
    const chosen = new Set<number>();
    while (chosen.size < 3)
      chosen.add(Math.floor(random() * RECOVERY_PHRASE_TOKENS));
    const positions = [...chosen].sort((left, right) => left - right);
    const kinds = new Set(positions.map(tokenKind));
    if (kinds.size === 2) return positions;
  }
}

/**
 * Whether a recall entry equals the phrase's token at `index`. Numbers compare
 * by value, as the agent parses them, so "032" matches "32".
 */
export function recallMatches(
  phrase: string,
  index: number,
  value: string,
): boolean {
  const expected = phrase.split(/\s+/)[index];
  const typed = normalizeToken(index, value);
  if (expected === undefined || !typed) return false;
  if (tokenKind(index) === 'number')
    return /^\d+$/.test(typed) && Number(typed) === Number(expected);
  return typed === expected;
}
