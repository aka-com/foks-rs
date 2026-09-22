import type { ReactNode, Dispatch, SetStateAction } from 'react';
import {
  Icon,
  Button,
  Chip,
  Inset,
  InsetRow,
  SheetDialog,
} from '../components';
import type { FirstRunCheckpoint, FirstRunStateName } from '../first-run-state';
import { Pane, Foot } from './first-run-view';
interface Props {
  state: 'protect' | 'phrase';
  checkpoint: FirstRunCheckpoint;
  busy: boolean;
  mutationBusy: boolean;
  recoveryPhrase: string | null;
  phraseConcealed: boolean;
  revealPhrase: () => void;
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
            Write down these 17 words and keep them somewhere other than this
            device. Anyone with them can recover your account.
          </p>
          {state === 'phrase' ? (
            <>
              <RecoveryPhrase
                phrase={recoveryPhrase}
                concealed={phraseConcealed}
                onReveal={revealPhrase}
              />
              <label className="local-confirm">
                <input
                  type="checkbox"
                  checked={phraseWritten}
                  onChange={(event) => setPhraseWritten(event.target.checked)}
                />
                <span>I have written down all 17 words.</span>
              </label>
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
              Write down these 17 words to recover your account if every device
              is lost.
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
            title="Save your recovery phrase"
            footer={
              <>
                <Button disabled={mutationBusy} onClick={() => go('protect')}>
                  Not now
                </Button>
                <Button
                  variant="primary"
                  disabled={!recoveryPhrase || !phraseWritten || busy}
                  onClick={() => void commitBackup()}
                >
                  {busy ? 'Saving...' : 'Done'}
                </Button>
              </>
            }
          >
            <>
              <p>
                Anyone with these words can access your account. Store them
                somewhere safe, like an offline vault, or another password
                manager.
              </p>
              <RecoveryPhrase
                phrase={recoveryPhrase}
                concealed={phraseConcealed}
                onReveal={revealPhrase}
              />
              <button
                type="button"
                aria-label="I have written this down"
                className={`check${phraseWritten ? ' on' : ''}`}
                onClick={() => setPhraseWritten((value) => !value)}
              >
                <span className="bx">{phraseWritten ? '✓' : ''}</span>I have
                written this down
              </button>
            </>
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
