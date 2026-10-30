/**
 * Shared modal dialog for displaying and confirming a newly generated recovery
 * phrase during first-run setup and on the Devices screen.
 */

import type { ReactNode } from 'react';
import { Button, Icon, SheetDialog } from '../components';

export interface RecoveryPhraseSheetProps {
  /** Space-separated recovery phrase words, or null while generation is in progress. */
  phrase: string | null;
  /** Whether to mask the recovery words until the user clicks reveal. */
  concealed?: boolean;
  onReveal?: () => void;
  /** Optional subtitle under the dialog title identifying the target account and server. */
  subtitle?: string;
  /** Whether closing the dialog permanently discards the displayed phrase. */
  oneTime?: boolean;
  written: boolean;
  onWrittenChange: (written: boolean) => void;
  busy: boolean;
  /** Whether Escape and the Not Now button can dismiss the dialog; false while saving. */
  dismissible?: boolean;
  onClose: () => void;
  /** Tooltip explaining why saving is unavailable. */
  saveTitle?: string;
  saveDisabled?: boolean;
  onSave: () => void;
}

export function RecoveryPhraseSheet({
  phrase,
  concealed = false,
  onReveal,
  subtitle,
  oneTime = false,
  written,
  onWrittenChange,
  busy,
  dismissible = true,
  onClose,
  saveTitle,
  saveDisabled = false,
  onSave,
}: RecoveryPhraseSheetProps): ReactNode {
  const count = phrase?.trim().split(/\s+/).length ?? 0;
  return (
    <SheetDialog
      width="wide"
      dismissible={dismissible}
      onClose={onClose}
      glyph={
        <span className="kic Store" aria-hidden="true">
          <Icon name="fileText" />
        </span>
      }
      title="Save Recovery Phrase"
      subtitle={subtitle}
      footer={
        <>
          <Button disabled={!dismissible} onClick={onClose}>
            Not Now
          </Button>
          <Button
            variant="primary"
            title={saveTitle}
            disabled={!phrase || !written || busy || saveDisabled}
            onClick={onSave}
          >
            {busy ? 'Saving…' : 'Done'}
          </Button>
        </>
      }
    >
      <p>
        Anyone with these words can access your account. Store them somewhere
        safe, like an offline vault or another password manager.
        {oneTime
          ? ' The phrase cannot be shown again after you close this.'
          : ''}
      </p>
      <RecoveryPhraseWords
        phrase={phrase}
        concealed={concealed}
        onReveal={onReveal}
      />
      <label className="local-confirm">
        <input
          type="checkbox"
          checked={written}
          onChange={(event) => onWrittenChange(event.target.checked)}
        />
        <span>
          {count > 1
            ? `I have written down all ${count} words.`
            : 'I have written down the phrase.'}
        </span>
      </label>
    </SheetDialog>
  );
}

/** Renders the numbered recovery phrase words, masking them behind a reveal button when `concealed` is true. */
export function RecoveryPhraseWords({
  phrase,
  concealed = false,
  onReveal,
}: {
  phrase: string | null;
  concealed?: boolean;
  onReveal?: () => void;
}): ReactNode {
  if (!phrase) return <p>Preparing your phrase…</p>;
  const words = phrase.trim().split(/\s+/);
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
      {concealed && onReveal ? (
        <Button onClick={onReveal}>Show Recovery Phrase</Button>
      ) : null}
    </>
  );
}
