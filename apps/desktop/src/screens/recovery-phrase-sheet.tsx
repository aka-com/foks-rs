/**
 * The one sheet that reveals a new recovery phrase, in first run and on
 * Devices: the phrase's words numbered in the monospace face, a warning, and
 * the confirmation that every word was written down, which enables saving.
 */

import type { ReactNode } from 'react';
import { Button, Icon, SheetDialog } from '../components';

export interface RecoveryPhraseSheetProps {
  /** The phrase to reveal; null while it is being prepared. */
  phrase: string | null;
  /** Whether the words are masked until the user asks to see them. */
  concealed?: boolean;
  onReveal?: () => void;
  /** One muted line under the title: whose account the phrase recovers. */
  subtitle?: string;
  /** The phrase is not kept anywhere once the sheet closes. */
  oneTime?: boolean;
  written: boolean;
  onWrittenChange: (written: boolean) => void;
  busy: boolean;
  /** Escape and Not Now close the sheet; false while a save runs. */
  dismissible?: boolean;
  onClose: () => void;
  /** Why saving is refused, when it is. */
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

/** A recovery phrase's numbered words, masked until revealed when asked. */
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
