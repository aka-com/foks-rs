/**
 * Test recovery phrase: the written copy typed back into 17 fields and
 * compared, on this device, with the account's backup keys.
 *
 * The fields alternate word and number in the order of the written copy and
 * check each token as it is typed, completing words from the BIP-39 list. The
 * comparison itself is the agent's: it derives the key and answers whether
 * the account holds it, has revoked it, or holds another key whose public name
 * (the first word and number) the copy shares. Nothing typed here is sent to
 * the server or stored; a match records only the time, on this device.
 */

import { useEffect, useId, useRef, useState } from 'react';
import type {
  ClipboardEvent,
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
} from 'react';
import type { BackupEnrollment, Bridge, RecoveryPhraseCheck } from '../bridge';
import { normalizeCommandError } from '../bridge';
import { Button, Chip, Icon, SheetDialog } from '../components';
import type { AccountStore } from '../model';
import { useSheetGuard } from '../navigation-guard';
import {
  RECOVERY_PHRASE_TOKENS,
  TOKEN_PROBLEM_TEXT,
  completions,
  joinTokens,
  normalizeToken,
  spreadTokens,
  tokenComplete,
  tokenKind,
  tokenLabel,
  tokenProblem,
} from '../recovery-phrase';
import { recordRecoveryPhraseTest } from '../recovery-phrase-tests';
import { useConcealOnInactive } from '../use-conceal-on-inactive';

const EMPTY: readonly string[] = Array.from(
  { length: RECOVERY_PHRASE_TOKENS },
  () => '',
);

/** The open completion list: which field it belongs to and the chosen row. */
interface Suggestions {
  index: number;
  items: string[];
  active: number;
}

/** "paper-backup (cage 32)": the local alias and the key's public name. */
function keyName(check: RecoveryPhraseCheck): string {
  if (check.backupAlias && check.matchedName)
    return `${check.backupAlias} (${check.matchedName})`;
  return check.backupAlias ?? check.matchedName ?? 'an unnamed recovery phrase';
}

export function RecoveryPhraseTestSheet({
  bridge,
  store,
  username,
  backups,
  onClose,
  onError,
}: {
  bridge: Bridge;
  store: AccountStore;
  /** The account's username, which the intro names. */
  username: string;
  /** The recovery phrases this device lists for the account. */
  backups: readonly BackupEnrollment[];
  onClose: () => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const id = useId();
  const [tokens, setTokens] = useState<readonly string[]>(EMPTY);
  const [left, setLeft] = useState<ReadonlySet<number>>(new Set());
  const [focused, setFocused] = useState<number | null>(null);
  const [suggestions, setSuggestions] = useState<Suggestions | null>(null);
  const [busy, setBusy] = useState(false);
  // Typed tokens are masked while the window is inactive, as a revealed
  // phrase is; focusing a field shows them again.
  const [concealed, setConcealed] = useState(false);
  useConcealOnInactive(() => setConcealed(true));
  // The field to focus once the grid is drawn again after a result.
  const [focusRequest, setFocusRequest] = useState<number | null>(null);
  const [refused, setRefused] = useState<string | null>(null);
  const [result, setResult] = useState<RecoveryPhraseCheck | null>(null);
  // The first two tokens of the copy last tested, for a result that names
  // them; they are the phrase's public name, not secret material.
  const [testedName, setTestedName] = useState('');
  // Whether a match's time reached local storage on this device.
  const [recorded, setRecorded] = useState(false);
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the recovery phrase test.' }
      : null,
  );

  const problemAt = (index: number): string | null => {
    const problem = tokenProblem(
      index,
      tokens[index] ?? '',
      left.has(index) && focused !== index,
    );
    return problem ? TOKEN_PROBLEM_TEXT[problem] : null;
  };
  const problems = tokens.map((_, index) => problemAt(index));
  const fix = problems.filter(Boolean).length;
  const entered = tokens.filter((token, index) =>
    tokenComplete(index, token),
  ).length;
  const skipped = tokens.some((token, index) => {
    const problem = tokenProblem(index, token, true);
    return problem === 'expected-word' || problem === 'expected-number';
  });
  const ready = entered === RECOVERY_PHRASE_TOKENS;

  const focusField = (index: number): void => {
    const target = inputs.current[Math.max(0, Math.min(index, 16))];
    target?.focus();
  };
  const setToken = (index: number, value: string): void => {
    const next = [...tokens];
    next[index] = normalizeToken(index, value);
    setTokens(next);
    setRefused(null);
    const typed = next[index] ?? '';
    const items = tokenKind(index) === 'word' ? completions(typed) : [];
    // A finished word needs no list unless longer words share it as a prefix.
    const finished = items.length === 1 && items[0] === typed;
    setSuggestions(
      items.length && !finished ? { index, items, active: 0 } : null,
    );
  };
  const accept = (index: number, word: string): void => {
    const next = [...tokens];
    next[index] = word;
    setTokens(next);
    setSuggestions(null);
    focusField(index + 1);
  };
  const keyDown = (
    index: number,
    event: ReactKeyboardEvent<HTMLInputElement>,
  ): void => {
    const list = suggestions?.index === index ? suggestions : null;
    if (list && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
      event.preventDefault();
      const step = event.key === 'ArrowDown' ? 1 : -1;
      setSuggestions({
        ...list,
        active: (list.active + step + list.items.length) % list.items.length,
      });
      return;
    }
    if (list && event.key === 'Tab' && event.shiftKey) {
      // Moving back leaves the field as typed.
      setSuggestions(null);
      return;
    }
    if (
      list &&
      (event.key === 'Enter' || event.key === 'Tab' || event.key === ' ')
    ) {
      const word = list.items[list.active];
      if (word) {
        event.preventDefault();
        accept(index, word);
      }
      return;
    }
    if (list && event.key === 'Escape') {
      // The first Escape closes the list; the next one closes the sheet.
      event.preventDefault();
      event.stopPropagation();
      setSuggestions(null);
      return;
    }
    if (event.key === ' ' || (event.key === 'Enter' && !ready)) {
      event.preventDefault();
      if (tokens[index]) focusField(index + 1);
      return;
    }
    if (event.key === 'Enter' && ready) {
      event.preventDefault();
      void submit();
      return;
    }
    if (event.key === 'Backspace' && !tokens[index] && index > 0) {
      event.preventDefault();
      focusField(index - 1);
    }
  };
  const paste = (
    index: number,
    event: ClipboardEvent<HTMLInputElement>,
  ): void => {
    const text = event.clipboardData.getData('text/plain');
    if (!/\s/.test(text.trim())) return;
    // A pasted phrase keeps its token boundaries, one token per field.
    event.preventDefault();
    const next = spreadTokens(tokens, index, text);
    setTokens(next);
    setSuggestions(null);
    setRefused(null);
    const count = text.split(/\s+/).filter(Boolean).length;
    setLeft(
      (current) =>
        new Set([
          ...current,
          ...Array.from({ length: count }, (_, offset) => index + offset),
        ]),
    );
    focusField(index + count);
  };

  const submit = async (): Promise<void> => {
    if (!ready || busy) return;
    setBusy(true);
    setSuggestions(null);
    setRefused(null);
    const name = `${tokens[0] ?? ''} ${tokens[1] ?? ''}`;
    try {
      const check = await bridge.verifyRecoveryPhrase(
        store.id,
        joinTokens(tokens),
      );
      setTestedName(name);
      setResult(check);
      if (check.matchedBackupId && !check.revoked) {
        setRecorded(recordRecoveryPhraseTest(check.matchedBackupId));
        // A matched copy has done its job; nothing typed is kept.
        setTokens(EMPTY);
        setLeft(new Set());
      }
    } catch (error) {
      const normalized = normalizeCommandError(error);
      if (normalized.code === 'invalid-request') setRefused(normalized.message);
      else onError(error);
    } finally {
      setBusy(false);
    }
  };
  const editPhrase = (): void => {
    const nearMiss = result?.matchedBackupId === null && !!result.matchedName;
    setResult(null);
    // A near miss already agrees on positions 1 and 2.
    setFocusRequest(nearMiss ? 2 : 0);
  };
  const testAnother = (): void => {
    setResult(null);
    setTokens(EMPTY);
    setLeft(new Set());
    setFocusRequest(0);
  };
  useEffect(() => {
    if (focusRequest === null || result) return;
    inputs.current[focusRequest]?.focus();
    setFocusRequest(null);
  }, [focusRequest, result]);

  const footer = result ? (
    result.matchedBackupId && !result.revoked ? (
      <Button variant="primary" onClick={onClose}>
        Done
      </Button>
    ) : result.matchedBackupId ? (
      <>
        <Button onClick={onClose}>Close</Button>
        <Button variant="primary" onClick={testAnother}>
          Test another phrase
        </Button>
      </>
    ) : (
      <>
        <span className="phrase-test-count">
          Cleared when this sheet closes.
        </span>
        <Button onClick={onClose}>Close</Button>
        <Button variant="primary" onClick={editPhrase}>
          Edit phrase
        </Button>
      </>
    )
  ) : (
    <>
      <span className="phrase-test-count" aria-live="polite">
        <b>
          {entered} of {RECOVERY_PHRASE_TOKENS}
        </b>{' '}
        entered
        {fix ? ` · ${fix} ${fix === 1 ? 'field' : 'fields'} to fix` : ''}
      </span>
      <Button disabled={busy} onClick={onClose}>
        Cancel
      </Button>
      <Button
        variant="primary"
        disabled={!ready || busy}
        title={ready ? undefined : 'Enter all 17 words and numbers first'}
        onClick={() => void submit()}
      >
        {busy ? 'Checking…' : 'Test phrase'}
      </Button>
    </>
  );

  return (
    <SheetDialog
      width="wide"
      onClose={onClose}
      dismissible={!busy}
      glyph={
        <span className="server-mark">
          <Icon name="shield" />
        </span>
      }
      title="Test recovery phrase"
      footer={footer}
    >
      {result ? (
        <RecoveryPhraseResult
          check={result}
          testedName={testedName}
          recorded={recorded}
          username={username}
          backups={backups}
        />
      ) : (
        <>
          <p id={`${id}-intro`}>
            Type the phrase from your written copy, not from memory. FOKS checks
            it on this device against the recovery phrases on <b>{username}</b>
            ’s account. It is not sent to the server, and it is cleared when
            this sheet closes.
          </p>
          <div
            className="phrase-test-grid"
            role="group"
            aria-label="Recovery phrase, 17 words and numbers"
            aria-describedby={`${id}-intro`}
          >
            {tokens.map((token, index) => {
              const word = tokenKind(index) === 'word';
              const problem = problems[index];
              const list = suggestions?.index === index ? suggestions : null;
              const listId = `${id}-list-${index}`;
              return (
                <div
                  key={index}
                  className={[
                    'phrase-test-cell',
                    word ? 'word-slot' : 'number-slot',
                    problem ? 'bad' : tokenComplete(index, token) ? 'good' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                >
                  <span className="phrase-test-box">
                    <label
                      className="phrase-test-position"
                      htmlFor={`${id}-${index}`}
                      aria-hidden="true"
                    >
                      {index + 1}
                    </label>
                    <input
                      ref={(element) => {
                        inputs.current[index] = element;
                      }}
                      id={`${id}-${index}`}
                      type={concealed ? 'password' : 'text'}
                      autoComplete="off"
                      autoCapitalize="off"
                      spellCheck={false}
                      inputMode={word ? undefined : 'numeric'}
                      placeholder={word ? 'word' : '0–8191'}
                      aria-label={`Position ${index + 1}, ${tokenLabel(index).toLowerCase()} of ${word ? 9 : 8}`}
                      aria-invalid={problem ? true : undefined}
                      aria-describedby={`${id}-error-${index}`}
                      {...(word
                        ? {
                            role: 'combobox',
                            'aria-autocomplete': 'list' as const,
                            'aria-expanded': list !== null,
                            'aria-controls': listId,
                            'aria-activedescendant': list
                              ? `${listId}-${list.active}`
                              : undefined,
                          }
                        : {})}
                      value={token}
                      onChange={(event) => setToken(index, event.target.value)}
                      onKeyDown={(event) => keyDown(index, event)}
                      onPaste={(event) => paste(index, event)}
                      onFocus={() => {
                        setFocused(index);
                        setConcealed(false);
                      }}
                      onBlur={() => {
                        setFocused(null);
                        setLeft((current) => new Set([...current, index]));
                        setSuggestions((current) =>
                          current?.index === index ? null : current,
                        );
                      }}
                    />
                  </span>
                  <span
                    className="phrase-test-error"
                    id={`${id}-error-${index}`}
                  >
                    {problem}
                  </span>
                  {list ? (
                    <ul
                      className="phrase-test-suggestions"
                      role="listbox"
                      id={listId}
                      aria-label="Matching words"
                    >
                      {list.items.map((item, at) => (
                        <li
                          key={item}
                          id={`${listId}-${at}`}
                          role="option"
                          aria-selected={at === list.active}
                          className={at === list.active ? 'on' : undefined}
                          // Mouse down keeps focus in the field until the
                          // choice is applied.
                          onMouseDown={(event) => {
                            event.preventDefault();
                            accept(index, item);
                          }}
                        >
                          {item}
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </div>
              );
            })}
          </div>
          {skipped ? (
            <p className="phrase-test-hint">
              Words and numbers alternate. A word in a number field, or a number
              in a word field, usually means a token was skipped or repeated.
            </p>
          ) : null}
          {refused ? <p className="crit">{refused}</p> : null}
        </>
      )}
    </SheetDialog>
  );
}

function RecoveryPhraseResult({
  check,
  testedName,
  recorded,
  username,
  backups,
}: {
  check: RecoveryPhraseCheck;
  testedName: string;
  /** Whether a match's time was stored on this device. */
  recorded: boolean;
  username: string;
  backups: readonly BackupEnrollment[];
}): ReactNode {
  const name = keyName(check);
  if (check.matchedBackupId && !check.revoked)
    return (
      <>
        <Outcome tone="ok" title={`Matches recovery phrase ${name}`}>
          This copy can recover {username}’s account if every device is lost.
          Keep it somewhere other than this device.
        </Outcome>
        <div className="inset settings-inset phrase-test-facts">
          <div className="fr">
            <span className="k">Recovery phrase</span>
            <span className="v">{name}</span>
          </div>
          <div className="fr">
            <span className="k">Last tested</span>
            <span className="v">
              {recorded ? 'Today, on this device' : 'Not recorded'}
            </span>
          </div>
        </div>
        <p className="phrase-test-hint">
          The phrase you typed has been cleared.
          {recorded
            ? ''
            : ' This device could not store the time of this test, so Devices still shows the previous result.'}
        </p>
      </>
    );
  if (check.matchedBackupId)
    return (
      <>
        <Outcome
          tone="stop"
          title={`This is recovery phrase ${name}, which was revoked`}
        >
          It can no longer recover this account. Destroy this copy so it is not
          mistaken for a working one.
        </Outcome>
        <div className="inset settings-inset phrase-test-facts">
          <div className="fr">
            <span className="k">Recovery phrase</span>
            <span className="v">
              {name} <Chip tone="bad">Revoked</Chip>
            </span>
          </div>
        </div>
      </>
    );
  if (check.matchedName)
    return (
      <>
        <Outcome
          tone="warn"
          title={`This phrase starts like ${name} but does not match it`}
        >
          Positions 1 and 2 match the name {check.matchedName}, so this is
          probably your copy of it. At least one word or number in positions 3
          to 17 is different, and this copy could not recover the account.
          {check.revoked
            ? ' That phrase has been revoked, so even a corrected copy cannot recover this account.'
            : ''}
        </Outcome>
        <ol
          className="phrase-test-strip"
          aria-label="Positions 1 and 2 match; check positions 3 to 17"
        >
          {Array.from({ length: RECOVERY_PHRASE_TOKENS }, (_, index) => (
            <li
              key={index}
              className={index < 2 ? 'ok' : 'check'}
              aria-hidden="true"
            >
              {index + 1}
            </li>
          ))}
        </ol>
        <p className="phrase-test-hint">
          Check words and numbers 3 to 17 against your written copy and test
          again. If the written copy itself is wrong, create a new recovery
          phrase from Add a device and revoke {name}.
        </p>
      </>
    );
  return (
    <>
      <Outcome
        tone="stop"
        title="This phrase does not match any recovery phrase on this account"
      >
        No recovery phrase on {username}’s account starts with{' '}
        <b>{testedName}</b>. Positions 1 and 2 are a phrase’s name, so check
        them against your copy first.
      </Outcome>
      <div className="inset settings-inset phrase-test-facts">
        <div className="fr">
          <span className="k">Starts with</span>
          <span className="v">
            <code>{testedName}</code>
          </span>
        </div>
        <div className="fr">
          <span className="k">On this account</span>
          <span className="v">
            {backups.length
              ? backups.map((backup) => backup.backupAlias).join(', ')
              : 'No recovery phrase listed on this device'}
          </span>
        </div>
      </div>
      <p className="phrase-test-hint">
        The comparison uses this account as this device last verified it. A
        recovery phrase created on another device is included after this account
        is synchronized here.
      </p>
    </>
  );
}

function Outcome({
  tone,
  title,
  children,
}: {
  tone: 'ok' | 'warn' | 'stop';
  title: string;
  children: ReactNode;
}): ReactNode {
  return (
    <div className={`phrase-test-result ${tone}`} role="status">
      <Icon name={tone === 'ok' ? 'check' : 'alert'} />
      <div className="t">
        <h3>{title}</h3>
        <p>{children}</p>
      </div>
    </div>
  );
}
