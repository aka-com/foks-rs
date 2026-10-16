import { useCallback, useRef, useState } from 'react';
import type { Bridge } from '../../bridge';
import type { protectionNavigationPolicy } from './session-transitions';

/**
 * Owns the recoverable preparation promise and the currently visible phrase.
 * Effect cleanup retires its publisher without cancelling the server operation;
 * a later effect can attach to that same preparation, while conceal/reset drops it.
 */
export function useRecoveryBackup() {
  const [generatedRecoveryPhrase, setGeneratedRecoveryPhrase] = useState<
    string | null
  >(null);
  const [recoveryPhraseConcealed, setRecoveryPhraseConcealed] = useState(false);
  const [phraseWritten, setPhraseWritten] = useState(false);
  const [phraseOperation, setPhraseOperation] = useState<symbol | null>(null);
  const phraseOwner = useRef<symbol | null>(null);
  const backupPreparation = useRef<{
    key: string;
    promise: Promise<{ backupAlias: string; phrase: string }>;
  } | null>(null);

  const clearBackup = useCallback(() => {
    phraseOwner.current = null;
    setPhraseOperation(null);
    backupPreparation.current = null;
    setGeneratedRecoveryPhrase(null);
    setRecoveryPhraseConcealed(false);
    setPhraseWritten(false);
  }, []);

  const navigateBackup = useCallback(
    (policy: ReturnType<typeof protectionNavigationPolicy>) => {
      if (policy.revealPhrase) setRecoveryPhraseConcealed(false);
      if (policy.discardBackup) {
        backupPreparation.current = null;
        setGeneratedRecoveryPhrase(null);
      }
      if (policy.resetAcknowledgement) setPhraseWritten(false);
    },
    [],
  );

  const prepare = useCallback(
    (
      bridge: Bridge,
      profile: string,
      accountAlias: string,
      onFailure: (error: unknown) => void,
    ): (() => void) => {
      const owner = Symbol('phrase preparation');
      phraseOwner.current = owner;
      setPhraseOperation(owner);
      const key = `${profile}\u0000${accountAlias}`;
      const attempt =
        backupPreparation.current?.key === key
          ? backupPreparation.current.promise
          : bridge.prepareOwnerBackup(profile, accountAlias, 'paper');
      backupPreparation.current = { key, promise: attempt };
      void attempt.then(
        (result) => {
          if (phraseOwner.current === owner) {
            setGeneratedRecoveryPhrase(result.phrase);
            setPhraseOperation(null);
          }
        },
        (error) => {
          if (phraseOwner.current !== owner) return;
          backupPreparation.current = null;
          setPhraseOperation(null);
          onFailure(error);
        },
      );
      return () => {
        if (phraseOwner.current === owner) phraseOwner.current = null;
        setPhraseOperation((current) => (current === owner ? null : current));
      };
    },
    [],
  );

  return {
    generatedRecoveryPhrase,
    recoveryPhraseConcealed,
    setRecoveryPhraseConcealed,
    phraseWritten,
    setPhraseWritten,
    phraseOperation,
    prepare,
    clearBackup,
    navigateBackup,
  };
}
