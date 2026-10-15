import assert from 'node:assert/strict';
import test from 'node:test';
import { installDom } from './lib/dom-harness';
import {
  CHARACTER_SETS,
  DEFAULT_GENERATOR_POLICY,
  PASSPHRASE_WORDS,
  characterPool,
  generatePassword,
  parseGeneratorPolicy,
  randomIndex,
  rememberGeneratorPolicy,
  storedGeneratorPolicy,
  strengthOfBits,
  typedPasswordStrength,
} from '../src/password-generator';
import type { RandomFill } from '../src/password-generator';
installDom({ url: 'http://localhost/' });

/** A fill that returns `values` in order, one per draw. */
function scripted(values: readonly number[]): {
  fill: RandomFill;
  draws: () => number;
} {
  let next = 0;
  return {
    fill: (target) => {
      assert.ok(next < values.length, 'the script has a value left');
      target[0] = values[next++];
    },
    draws: () => next,
  };
}

test('a draw at or above the largest multiple of the bound is rejected', () => {
  // 2^32 = 3 × 1431655765 + 1, so only 2^32 - 1 is out of range for 3.
  const top = 0xffff_ffff;
  const script = scripted([top, top, 4]);
  assert.equal(randomIndex(3, script.fill), 1);
  assert.equal(script.draws(), 3);

  // For 10 the accepted range ends at 4294967290.
  const ten = scripted([4_294_967_290, 4_294_967_289]);
  assert.equal(randomIndex(10, ten.fill), 9);
  assert.equal(ten.draws(), 2);

  assert.throws(() => randomIndex(0), RangeError);
  assert.throws(() => randomIndex(1.5), RangeError);
});

test('every index of a small bound is reachable and none falls outside it', () => {
  const seen = new Set<number>();
  for (let draw = 0; draw < 2_000; draw++) {
    const index = randomIndex(7);
    assert.ok(index >= 0 && index < 7);
    seen.add(index);
  }
  assert.equal(seen.size, 7);
});

test('the character pool follows the selected sets and leaves out ambiguous characters', () => {
  const pool = characterPool(DEFAULT_GENERATOR_POLICY);
  for (const character of '0O1lI')
    assert.equal(pool.includes(character), false);
  assert.equal(new Set(pool).size, pool.length);
  assert.equal(pool.length, 26 + 26 + 10 + 13 - 5);

  const digits = characterPool({
    ...DEFAULT_GENERATOR_POLICY,
    sets: { upper: false, lower: false, digits: true, symbols: false },
    avoidAmbiguous: false,
  });
  assert.equal(digits, CHARACTER_SETS.digits);
});

test('a generated password has the asked length, its pool and exact entropy', () => {
  const policy = { ...DEFAULT_GENERATOR_POLICY, length: 40 };
  const pool = characterPool(policy);
  const generated = generatePassword(policy);
  assert.equal(generated.value.length, 40);
  for (const character of generated.value)
    assert.ok(pool.includes(character), `${character} is in the pool`);
  assert.equal(generated.bits, 40 * Math.log2(pool.length));
  assert.notEqual(generatePassword(policy).value, generated.value);

  // The script picks the first and the last character of the pool.
  const script = scripted([0, pool.length - 1]);
  assert.equal(
    generatePassword({ ...policy, length: 2 }, script.fill).value,
    `${pool[0]}${pool[pool.length - 1]}`,
  );
});

test('a passphrase draws whole words and joins them with the separator', () => {
  const policy = {
    ...DEFAULT_GENERATOR_POLICY,
    mode: 'passphrase' as const,
    words: 6,
    separator: ' ' as const,
  };
  const generated = generatePassword(policy);
  const words = generated.value.split(' ');
  assert.equal(words.length, 6);
  for (const word of words) assert.ok(PASSPHRASE_WORDS.includes(word));
  assert.equal(PASSPHRASE_WORDS.length, 2048);
  assert.equal(generated.bits, 66);

  const script = scripted([0, 2047, 5, 6]);
  assert.equal(
    generatePassword({ ...policy, words: 4, separator: '-' }, script.fill)
      .value,
    [0, 2047, 5, 6].map((index) => PASSPHRASE_WORDS[index]).join('-'),
  );
});

test('strength levels follow the entropy of a generated password', () => {
  assert.deepEqual(strengthOfBits(147.4), {
    level: 4,
    text: 'Strong · 147 bits',
    weak: false,
  });
  assert.equal(strengthOfBits(55).level, 3);
  assert.equal(strengthOfBits(44).level, 2);
  assert.deepEqual(strengthOfBits(24), {
    level: 1,
    text: 'Weak · 24 bits',
    weak: true,
  });
});

test('a typed password is warned about at level 1; without a warning it is not rated', () => {
  assert.equal(typedPasswordStrength(''), null);
  assert.deepEqual(typedPasswordStrength('Password1!'), {
    level: 1,
    text: 'Weak · a common password',
    weak: true,
  });
  assert.equal(
    typedPasswordStrength('P@55w0rd')?.text,
    'Weak · a common password',
  );
  // A well-known passphrase matches with its separators removed.
  assert.equal(
    typedPasswordStrength('correct horse battery staple')?.text,
    'Weak · a common password',
  );
  assert.equal(
    typedPasswordStrength('MyGithub-2024!', ['github.com', 'satoshi'])?.text,
    'Weak · contains the site or user name',
  );
  assert.equal(
    typedPasswordStrength('satoshi:river', ['https://example.org', 'satoshi'])
      ?.text,
    'Weak · contains the site or user name',
  );
  assert.equal(
    typedPasswordStrength('k7#Qz')?.text,
    'Weak · shorter than 8 characters',
  );
  const plain = typedPasswordStrength('violet-anchor-quarry-71', [
    'github.com',
    'me',
  ]);
  assert.deepEqual(plain, {
    level: null,
    text: 'Not measured · no common weakness found',
    weak: false,
  });
});

test('the site check uses the registrable label of the host, not a subdomain', () => {
  const site = 'Weak · contains the site or user name';
  assert.equal(
    typedPasswordStrength('Slack-is-great-2024!', ['app.slack.com', 'bob'])
      ?.text,
    site,
  );
  assert.equal(
    typedPasswordStrength('Google-rocks-9182', [
      'https://accounts.google.com/signin',
      'alice',
    ])?.text,
    site,
  );
  assert.equal(
    typedPasswordStrength('Barclays!Spring77', [
      'https://online.barclays.co.uk:8443/',
      '',
    ])?.text,
    site,
  );
  // An email user name contributes its local part.
  assert.equal(
    typedPasswordStrength('ringo.starr-1940', ['example.com', 'ringo@x.org'])
      ?.text,
    site,
  );
  // Each part of a dotted user name is checked, with or without a domain.
  for (const userName of ['john.smith@x.org', 'john.smith'])
    for (const value of ['John-1985!x', 'Smith-blue-ocean'])
      assert.equal(
        typedPasswordStrength(value, ['example.com', userName])?.text,
        site,
        `${value} ${userName}`,
      );
  // A subdomain is not the site, also under a country code whose
  // registrable name is short.
  for (const [value, host] of [
    ['login-violet-quarry', 'login.example.com'],
    ['login-violet-quarry', 'https://login.web.de'],
    ['Banking-2024-xyz!', 'https://banking.ing.de'],
    ['account-blue-ocean', 'account.n26.de'],
  ])
    assert.equal(
      typedPasswordStrength(value, [host, ''])?.text,
      'Not measured · no common weakness found',
      host,
    );
});

test('repeated characters and sequences are warned about', () => {
  const context = ['example.com', ''];
  for (const value of [
    '11111111',
    '00000000',
    '1111111111',
    'aaaaaaaa',
    'zzzzzzzzzzzz',
    '12341234',
    'asdfasdf',
    'aaaaaaaa1!',
    'aaaa1234!',
  ])
    assert.equal(
      typedPasswordStrength(value, context)?.text,
      'Weak · repeated characters',
      value,
    );
  for (const value of [
    'abcdefgh',
    'qwertyui',
    'zyxwvuts',
    '987654321',
    'ytrewq12',
    'abcd1234',
    'abcd1234!',
    'abcd1234!!',
    'poiuytre',
    'hgfedcba',
  ])
    assert.equal(
      typedPasswordStrength(value, context)?.text,
      'Weak · a sequence',
      value,
    );
  // A run inside a longer value, or before a longer random tail, is not one.
  for (const value of ['liberty-property-9', 'qwer8#4!9$2&7^1', 'abcz1234'])
    assert.equal(
      typedPasswordStrength(value, context)?.text,
      'Not measured · no common weakness found',
      value,
    );
});

test('stored settings are validated field by field and survive a round trip', () => {
  assert.deepEqual(parseGeneratorPolicy(null), DEFAULT_GENERATOR_POLICY);
  assert.deepEqual(parseGeneratorPolicy('not json'), DEFAULT_GENERATOR_POLICY);
  assert.deepEqual(
    parseGeneratorPolicy(
      JSON.stringify({
        mode: 'passphrase',
        length: 500,
        sets: { upper: false, lower: false, digits: false, symbols: false },
        avoidAmbiguous: 'yes',
        words: 2,
        separator: '/',
      }),
    ),
    {
      ...DEFAULT_GENERATOR_POLICY,
      mode: 'passphrase',
      length: 64,
      words: 4,
    },
  );

  const policy = {
    ...DEFAULT_GENERATOR_POLICY,
    length: 12,
    sets: { upper: false, lower: true, digits: true, symbols: false },
    separator: '_' as const,
  };
  try {
    rememberGeneratorPolicy(policy);
    assert.deepEqual(storedGeneratorPolicy(), policy);
  } finally {
    window.localStorage.removeItem('passwordGenerator');
  }
  assert.deepEqual(storedGeneratorPolicy(), DEFAULT_GENERATOR_POLICY);
});
