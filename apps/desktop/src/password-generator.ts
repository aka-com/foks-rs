/**
 * Password generation and strength feedback for the login editors.
 *
 * A generated password is drawn in the renderer, which already holds the
 * typed password in the New sheet and the login Edit form. Every character
 * or word is chosen with `crypto.getRandomValues` and rejection sampling, so
 * each is uniform over its pool and the entropy of the result is exactly
 * `length × log2(pool size)` bits.
 *
 * A typed or stored password has no such measure. It is checked for a few
 * common weaknesses (on a list of common passwords, containing the site or
 * user name, one short unit repeated, a sequence or keyboard row, too short)
 * and a warning is shown; nothing blocks saving it.
 *
 * The last generator settings are a per-device preference under the
 * `passwordGenerator` `localStorage` key. They never include a password.
 */

import { BIP39_ENGLISH } from './bip39-english';

export type GeneratorMode = 'characters' | 'passphrase';
export type CharacterSet = 'upper' | 'lower' | 'digits' | 'symbols';
export type PassphraseSeparator = '-' | ' ' | '.' | '_';

export interface GeneratorPolicy {
  mode: GeneratorMode;
  /** Characters in a random-character password. */
  length: number;
  /** The character sets a random-character password draws from. */
  sets: Readonly<Record<CharacterSet, boolean>>;
  /** Leave out characters that are easily misread: 0, O, 1, l and I. */
  avoidAmbiguous: boolean;
  /** Words in a passphrase. */
  words: number;
  separator: PassphraseSeparator;
}

export const LENGTH_RANGE = { min: 8, max: 64 } as const;
export const WORD_RANGE = { min: 4, max: 8 } as const;

export const CHARACTER_SETS: Readonly<Record<CharacterSet, string>> = {
  upper: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
  lower: 'abcdefghijklmnopqrstuvwxyz',
  digits: '0123456789',
  symbols: '!@#$%^&*-_=+?',
};

export const PASSPHRASE_SEPARATORS: readonly PassphraseSeparator[] = [
  '-',
  ' ',
  '.',
  '_',
];

const AMBIGUOUS = new Set(['0', 'O', '1', 'l', 'I']);

/**
 * Passphrase words: the BIP-39 English list the recovery phrase input
 * already embeds. 2,048 words give exactly 11 bits per word.
 */
export const PASSPHRASE_WORDS: readonly string[] = BIP39_ENGLISH;

export const DEFAULT_GENERATOR_POLICY: GeneratorPolicy = {
  mode: 'characters',
  length: 24,
  sets: { upper: true, lower: true, digits: true, symbols: true },
  avoidAmbiguous: true,
  words: 5,
  separator: '-',
};

/** Fills an array with random values, as `crypto.getRandomValues` does. */
export type RandomFill = (target: Uint32Array<ArrayBuffer>) => void;

const cryptoFill: RandomFill = (target) => {
  globalThis.crypto.getRandomValues(target);
};

/**
 * A uniform index in `[0, n)`. A 32-bit draw at or above the largest
 * multiple of `n` is rejected and drawn again, so no index is favoured by
 * the modulo.
 */
export function randomIndex(n: number, fill: RandomFill = cryptoFill): number {
  if (!Number.isInteger(n) || n < 1 || n > 0x1_0000_0000)
    throw new RangeError(`random index bound out of range: ${n}`);
  const limit = Math.floor(0x1_0000_0000 / n) * n;
  const draw = new Uint32Array(1);
  for (;;) {
    fill(draw);
    if (draw[0] < limit) return draw[0] % n;
  }
}

/** The characters a random-character password draws from, without repeats. */
export function characterPool(policy: GeneratorPolicy): string {
  const pool = (Object.keys(CHARACTER_SETS) as CharacterSet[])
    .filter((set) => policy.sets[set])
    .map((set) => CHARACTER_SETS[set])
    .join('');
  return policy.avoidAmbiguous
    ? [...pool].filter((character) => !AMBIGUOUS.has(character)).join('')
    : pool;
}

export interface GeneratedPassword {
  value: string;
  /** Exact entropy: every draw is uniform and independent. */
  bits: number;
}

export function generatePassword(
  policy: GeneratorPolicy,
  fill: RandomFill = cryptoFill,
): GeneratedPassword {
  const drawn: string[] = [];
  if (policy.mode === 'passphrase') {
    for (let index = 0; index < policy.words; index++)
      drawn.push(PASSPHRASE_WORDS[randomIndex(PASSPHRASE_WORDS.length, fill)]);
    return {
      value: drawn.join(policy.separator),
      bits: policy.words * Math.log2(PASSPHRASE_WORDS.length),
    };
  }
  const pool = characterPool(policy);
  if (!pool) throw new RangeError('no character set is selected');
  for (let index = 0; index < policy.length; index++)
    drawn.push(pool[randomIndex(pool.length, fill)]);
  return {
    value: drawn.join(''),
    bits: policy.length * Math.log2(pool.length),
  };
}

export type StrengthLevel = 1 | 2 | 3 | 4;

export interface Strength {
  /** 1 (weak) to 4 (strong); `null` when nothing is rated. */
  level: StrengthLevel | null;
  text: string;
  /** Shown as a warning. */
  weak: boolean;
}

const LEVEL_NAMES: Readonly<Record<StrengthLevel, string>> = {
  1: 'Weak',
  2: 'Fair',
  3: 'Good',
  4: 'Strong',
};

/** The rating of a generated password, from its exact entropy. */
export function strengthOfBits(bits: number): Strength {
  const level: StrengthLevel =
    bits < 36 ? 1 : bits < 50 ? 2 : bits < 70 ? 3 : 4;
  return {
    level,
    text: `${LEVEL_NAMES[level]} · ${Math.floor(bits)} bits`,
    weak: level === 1,
  };
}

/**
 * Lowercased passwords that head published breach frequency lists. A typed
 * password matches after case folding, the usual digit-for-letter
 * substitutions and a trailing run of digits and symbols are removed.
 */
const COMMON_PASSWORDS = new Set(
  (
    'password passw0rd 123456 1234567 12345678 123456789 1234567890 qwerty ' +
    'qwertyuiop asdfgh asdfghjkl zxcvbnm letmein welcome iloveyou admin ' +
    'administrator monkey dragon football baseball abc123 111111 000000 ' +
    '123123 654321 sunshine princess trustno1 hunter master shadow superman ' +
    'batman starwars changeme login secret 1q2w3e4r zaq12wsx passpass ' +
    'whatever freedom michael charlie jordan computer internet default ' +
    'access mustang pokemon correcthorsebatterystaple'
  ).split(' '),
);

const TRAILING_DIGITS_AND_SYMBOLS = /[\d!@#$%^&*?._+=-]+$/;

/** Folded forms of a typed password to look up in the common list. */
function commonForms(value: string): string[] {
  const lower = value.toLowerCase();
  const stem = lower.replace(TRAILING_DIGITS_AND_SYMBOLS, '');
  const unleet = (text: string, one: string): string =>
    text
      .replace(/0/g, 'o')
      .replace(/1/g, one)
      .replace(/!/g, 'i')
      .replace(/3/g, 'e')
      .replace(/[4@]/g, 'a')
      .replace(/[5$]/g, 's')
      .replace(/7/g, 't');
  return [
    lower,
    stem,
    // A passphrase with its separators removed.
    lower.replace(/[\s._-]+/g, ''),
    unleet(lower, 'i'),
    unleet(lower, 'l'),
    unleet(stem, 'i'),
    unleet(stem, 'l'),
    unleet(lower, 'i').replace(TRAILING_DIGITS_AND_SYMBOLS, ''),
    unleet(lower, 'l').replace(TRAILING_DIGITS_AND_SYMBOLS, ''),
  ];
}

/**
 * The registrable label of a host: the label before the top-level domain,
 * or before a two-letter country code and a second-level label of at most
 * three characters (`co.uk`, `com.au`). `app.slack.com` gives `slack`, so a
 * subdomain such as `login` or `accounts` is never taken for the site.
 */
function registrableLabel(host: string): string {
  const labels = host.split('.').filter(Boolean);
  if (labels.length < 2) return labels[0] ?? '';
  let index = labels.length - 2;
  if (
    index > 0 &&
    labels[index].length <= 3 &&
    labels[labels.length - 1].length === 2
  )
    index--;
  return labels[index];
}

/**
 * The parts of a site or user name a password should not contain: the
 * registrable label of a host, and the part before an `@` (a user name in a
 * URL, or the local part of an email address). Parts shorter than four
 * characters are left out.
 */
function contextWords(context: readonly string[]): string[] {
  return context
    .flatMap((entry) => {
      const authority = entry
        .trim()
        .toLowerCase()
        .replace(/^[a-z][a-z0-9+.-]*:\/\//, '')
        .split(/[/?#]/)[0];
      const at = authority.lastIndexOf('@');
      const host = authority.slice(at + 1).replace(/:\d*$/, '');
      return at < 0
        ? [registrableLabel(host)]
        : [authority.slice(0, at), registrableLabel(host)];
    })
    .filter((word) => word.length >= 4);
}

/** The whole value is one unit of one to four characters, repeated. */
const REPEATED_UNIT = /^(.{1,4})\1+$/su;

/** Keyboard rows, read left to right; a run is also checked reversed. */
const KEYBOARD_ROWS = ['1234567890', 'qwertyuiop', 'asdfghjkl', 'zxcvbnm'];

/**
 * The whole value is a run of four or more characters: consecutive letters
 * or digits, ascending or descending (`abcd`, `4321`), or part of a
 * keyboard row in either direction (`qwerty`, `lkjh`).
 */
function isSequence(text: string): boolean {
  const characters = [...text];
  if (characters.length < 4) return false;
  if (/^[a-z]+$|^\d+$/.test(text)) {
    const step = text.charCodeAt(1) - text.charCodeAt(0);
    if (
      (step === 1 || step === -1) &&
      characters.every(
        (_, index) =>
          index === 0 ||
          text.charCodeAt(index) - text.charCodeAt(index - 1) === step,
      )
    )
      return true;
  }
  const reversed = characters.reverse().join('');
  return KEYBOARD_ROWS.some(
    (row) => row.includes(text) || row.includes(reversed),
  );
}

/**
 * The feedback for a password not taken from the generator: a warning for
 * the first common weakness found, shown at level 1 (Weak), or a neutral
 * note without a level, since none of these checks measures entropy. The
 * repeat and sequence checks apply to the whole value and to the value
 * without a trailing run of digits and symbols (`abcd1234!`).
 */
export function typedPasswordStrength(
  value: string,
  context: readonly string[] = [],
): Strength | null {
  if (!value) return null;
  if (commonForms(value).some((form) => COMMON_PASSWORDS.has(form)))
    return { level: 1, text: 'Weak · a common password', weak: true };
  const lower = value.toLowerCase();
  if (contextWords(context).some((word) => lower.includes(word)))
    return {
      level: 1,
      text: 'Weak · contains the site or user name',
      weak: true,
    };
  // The stem is checked only when the run it drops is no longer than itself.
  const stem = lower.replace(TRAILING_DIGITS_AND_SYMBOLS, '');
  const shapes =
    stem && stem !== lower && lower.length <= 2 * stem.length
      ? [lower, stem]
      : [lower];
  if (shapes.some((shape) => REPEATED_UNIT.test(shape)))
    return { level: 1, text: 'Weak · repeated characters', weak: true };
  if (shapes.some(isSequence))
    return { level: 1, text: 'Weak · a sequence', weak: true };
  if ([...value].length < LENGTH_RANGE.min)
    return {
      level: 1,
      text: `Weak · shorter than ${LENGTH_RANGE.min} characters`,
      weak: true,
    };
  return {
    level: null,
    text: 'Not measured · no common weakness found',
    weak: false,
  };
}

const POLICY_KEY = 'passwordGenerator';

function clampInteger(
  value: unknown,
  range: { min: number; max: number },
  fallback: number,
): number {
  return typeof value === 'number' && Number.isInteger(value)
    ? Math.min(range.max, Math.max(range.min, value))
    : fallback;
}

/**
 * A policy from stored JSON. Each field that is missing or malformed takes
 * its default, and a selection with no character set falls back to all four.
 */
export function parseGeneratorPolicy(text: string | null): GeneratorPolicy {
  let stored: unknown;
  try {
    stored = text === null ? null : JSON.parse(text);
  } catch {
    stored = null;
  }
  if (!stored || typeof stored !== 'object') return DEFAULT_GENERATOR_POLICY;
  const record = stored as Record<string, unknown>;
  const defaults = DEFAULT_GENERATOR_POLICY;
  const storedSets =
    record.sets && typeof record.sets === 'object'
      ? (record.sets as Record<string, unknown>)
      : {};
  const sets = Object.fromEntries(
    (Object.keys(defaults.sets) as CharacterSet[]).map((set) => [
      set,
      typeof storedSets[set] === 'boolean'
        ? storedSets[set]
        : defaults.sets[set],
    ]),
  ) as Record<CharacterSet, boolean>;
  return {
    mode:
      record.mode === 'passphrase' || record.mode === 'characters'
        ? record.mode
        : defaults.mode,
    length: clampInteger(record.length, LENGTH_RANGE, defaults.length),
    sets: Object.values(sets).some(Boolean) ? sets : defaults.sets,
    avoidAmbiguous:
      typeof record.avoidAmbiguous === 'boolean'
        ? record.avoidAmbiguous
        : defaults.avoidAmbiguous,
    words: clampInteger(record.words, WORD_RANGE, defaults.words),
    separator: PASSPHRASE_SEPARATORS.includes(
      record.separator as PassphraseSeparator,
    )
      ? (record.separator as PassphraseSeparator)
      : defaults.separator,
  };
}

export function storedGeneratorPolicy(): GeneratorPolicy {
  try {
    return typeof window === 'undefined'
      ? DEFAULT_GENERATOR_POLICY
      : parseGeneratorPolicy(window.localStorage.getItem(POLICY_KEY));
  } catch {
    return DEFAULT_GENERATOR_POLICY;
  }
}

export function rememberGeneratorPolicy(policy: GeneratorPolicy): void {
  try {
    if (typeof window !== 'undefined')
      window.localStorage.setItem(POLICY_KEY, JSON.stringify(policy));
  } catch {
    // Restricted storage must not prevent generating a password.
  }
}
