import { useMemo, useState } from 'react';
import type { ReactNode, Dispatch, SetStateAction } from 'react';
import {
  Icon,
  Button,
  Chip,
  Inset,
  InsetRow,
  SheetDialog,
} from '../../components';
import type {
  FirstRunCheckpoint,
  FirstRunStateName,
} from '../../first-run-state';
import { RecallCheck, recallPositions } from '../devices';
import { Pane, Foot } from './first-run-view';
interface Props {
  state: 'protect' | 'phrase';
  checkpoint: FirstRunCheckpoint;
  busy: boolean;
  mutationBusy: boolean;
  recoveryPhrase: string | null;
  phraseConcealed: boolean;
  revealPhrase: () => void;
  /** Whether the recall check matched the written copy; it gates commit. */
  phraseWritten: boolean;
  setPhraseWritten: Dispatch<SetStateAction<boolean>>;
  go: (state: FirstRunStateName) => void;
  finishLocalProtection: (skip?: boolean) => Promise<void>;
  collapseLocalBackup: () => void;
  message: string | null;
  passphrase: string;
  confirmation: string;
  setPassphrase: (value: string) => void;
  setConfirmation: (value: string) => void;
  continueProtection: () => Promise<void>;
  commitBackup: () => Promise<void>;
}
export function RecoveryStep({
  state,
  checkpoint,
  busy,
  recoveryPhrase,
  phraseConcealed,
  revealPhrase,
  phraseWritten,
  setPhraseWritten,
  go,
  finishLocalProtection,
  collapseLocalBackup,
  message,
  passphrase,
  confirmation,
  setPassphrase,
  setConfirmation,
  continueProtection,
  mutationBusy,
  commitBackup,
}: Props): ReactNode {
  // The phrase is shown, then hidden while three of its tokens are typed back
  // from the written copy. The positions stay the same for one phrase.
  const [checking, setChecking] = useState(false);
  const positions = useMemo(
    () => (recoveryPhrase ? recallPositions() : []),
    [recoveryPhrase],
  );
  // A new phrase, or leaving the sheet (Not now, Esc), starts again at the
  // phrase itself rather than at a check of tokens not yet seen.
  const [shownPhrase, setShownPhrase] = useState(recoveryPhrase);
  if (shownPhrase !== recoveryPhrase) {
    setShownPhrase(recoveryPhrase);
    setChecking(false);
  }
  if (checking && state !== 'phrase') setChecking(false);
  const showPhraseAgain = (): void => {
    setChecking(false);
    setPhraseWritten(false);
  };
  const recall =
    recoveryPhrase && checking ? (
      <RecallCheck
        phrase={recoveryPhrase}
        positions={positions}
        unlocks={checkpoint.managedLocal ? 'Start using FOKS' : 'Done'}
        reveal={checkpoint.managedLocal ? 'Show the phrase again' : 'Back'}
        onMatched={setPhraseWritten}
      />
    ) : null;
  let content: ReactNode;
  if ((state === 'protect' || state === 'phrase') && checkpoint.managedLocal)
    content = (
      <Pane
        title="Recovery"
        header={false}
        foot={
          <Foot
            back={() => go(checkpoint.returning ? 'existing' : 'account')}
            note={
              <button
                className="lnk"
                onClick={() => void finishLocalProtection(true)}
              >
                Do this later
              </button>
            }
          >
            <Button
              variant="primary"
              disabled={
                busy ||
                (state === 'phrase' && (!recoveryPhrase || !phraseWritten))
              }
              onClick={() => void finishLocalProtection()}
            >
              Start using FOKS
            </Button>
          </Foot>
        }
      >
        <h1>Set up account recovery</h1>
        <p className="lead">
          Set up a recovery phrase now so you can recover your account if this
          device is lost.
        </p>
        <div className="local-recovery-card">
          <div className="local-recovery-head">
            <h2>Recovery phrase</h2>
            <Chip>Recommended</Chip>
          </div>
          <p>
            Write down these 17 words and numbers and keep them somewhere other
            than this device. Anyone with them can recover your account.
          </p>
          {state === 'phrase' && checking ? (
            <>
              {recall}
              <button
                className="lnk local-quiet-link"
                onClick={showPhraseAgain}
              >
                Show the phrase again
              </button>
            </>
          ) : state === 'phrase' ? (
            <>
              <RecoveryPhrase
                phrase={recoveryPhrase}
                concealed={phraseConcealed}
                onReveal={revealPhrase}
              />
              <Button
                disabled={!recoveryPhrase}
                onClick={() => setChecking(true)}
              >
                Check your copy
              </Button>
              <button
                className="lnk local-quiet-link"
                onClick={collapseLocalBackup}
              >
                Hide recovery phrase
              </button>
            </>
          ) : (
            <Button
              variant="primary"
              disabled={checkpoint.backupCommitted || busy}
              onClick={() => go('phrase')}
            >
              {checkpoint.backupCommitted
                ? 'Recovery phrase saved'
                : 'Show recovery phrase'}
            </Button>
          )}
        </div>
        <div className="local-other-protection">
          <div className="local-quiet-card">
            <b>Passphrase</b>
            <span>Add one later from Settings.</span>
          </div>
        </div>
        {message ? <p className="crit">{message}</p> : null}
      </Pane>
    );
  else
    content = (
      <Pane
        title="Save recovery phrase"
        header={false}
        scope="Configure account recovery"
        wide
        foot={
          <Foot back={() => go('account')}>
            <Button
              variant="primary"
              disabled={busy || passphrase !== confirmation}
              onClick={() => void continueProtection()}
            >
              Continue
            </Button>
          </Foot>
        }
      >
        <h1>Save recovery phrase</h1>
        <p className="lead">
          The keys controlling this account are only saved on this device. We
          recommend adding at least one recovery method now.
        </p>
        <div className="setup-grid">
          <div className="pcard">
            <h3>Recovery phrase</h3>
            <p>
              Write down these 17 words and numbers to recover your account if
              every device is lost.
            </p>
            <Button
              disabled={busy || (checkpoint.backupCommitted && !recoveryPhrase)}
              onClick={() => go('phrase')}
            >
              Generate a recovery phrase
            </Button>
          </div>
          <div className="pcard">
            <h3>Passphrase</h3>
            <p>
              Protects the keys stored on this device with a password. Optional.
            </p>
            <Inset>
              <InsetRow label="Passphrase">
                <input
                  type="password"
                  aria-label="Passphrase"
                  placeholder="••••••••••••"
                  value={passphrase}
                  onChange={(event) => setPassphrase(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="Confirm">
                <input
                  type="password"
                  aria-label="Confirm passphrase"
                  placeholder="••••••••••••"
                  value={confirmation}
                  onChange={(event) => setConfirmation(event.target.value)}
                />
              </InsetRow>
            </Inset>
          </div>
        </div>
        {message ? <p className="crit">{message}</p> : null}
        {state === 'phrase' ? (
          <SheetDialog
            width="wide"
            dismissible={!mutationBusy}
            onClose={() => go('protect')}
            glyph={<Icon name="key" />}
            title={checking ? 'Check your copy' : 'Save your recovery phrase'}
            step={checking ? 'Step 2 of 2' : 'Step 1 of 2'}
            footer={
              checking ? (
                <>
                  <Button disabled={mutationBusy} onClick={showPhraseAgain}>
                    Back
                  </Button>
                  <Button
                    variant="primary"
                    disabled={!recoveryPhrase || !phraseWritten || busy}
                    onClick={() => void commitBackup()}
                  >
                    {busy ? 'Saving...' : 'Done'}
                  </Button>
                </>
              ) : (
                <>
                  <Button disabled={mutationBusy} onClick={() => go('protect')}>
                    Not now
                  </Button>
                  <Button
                    variant="primary"
                    disabled={!recoveryPhrase || busy}
                    onClick={() => setChecking(true)}
                  >
                    Next
                  </Button>
                </>
              )
            }
          >
            {checking ? (
              recall
            ) : (
              <>
                <p>
                  Write down all 17 words and numbers, in order. Anyone with
                  them can access your account, so store the copy somewhere
                  safe, like an offline vault or another password manager.
                </p>
                <RecoveryPhrase
                  phrase={recoveryPhrase}
                  concealed={phraseConcealed}
                  onReveal={revealPhrase}
                />
                <p className="recall-hint">
                  Next, you will type three of these from your written copy.
                </p>
              </>
            )}
          </SheetDialog>
        ) : null}
      </Pane>
    );

  return content;
}

function RecoveryPhrase({
  phrase,
  concealed,
  onReveal,
}: {
  phrase: string | null;
  concealed: boolean;
  onReveal: () => void;
}): ReactNode {
  if (!phrase) return <p>Preparing your phrase…</p>;
  const words = phrase.split(/\s+/);
  return (
    <>
      <div
        className="words"
        aria-label={concealed ? 'Recovery phrase hidden' : undefined}
      >
        {words.map((word, index) => (
          <div
            className="word"
            key={`${index}-${concealed ? 'hidden' : word}`}
            aria-hidden={concealed || undefined}
          >
            <i>{index + 1}</i>
            {concealed ? '••••••' : word}
          </div>
        ))}
      </div>
      {concealed ? (
        <Button onClick={onReveal}>Show recovery phrase</Button>
      ) : null}
    </>
  );
}
