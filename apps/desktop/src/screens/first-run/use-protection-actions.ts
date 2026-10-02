import type { Bridge } from '../../bridge';
import { transitionFirstRun } from '../../first-run-state';
import type { FirstRunCheckpoint } from '../../first-run-state';
import type { useFirstRunController } from '../../use-first-run-controller';
import type { useSetupReconciliation } from './use-setup-reconciliation';

/** Account protection commands: UI drafts are ephemeral; only receipts enter the checkpoint. */
export function useProtectionActions({
  bridge,
  agentReady,
  checkpoint,
  accountAlias,
  passphrase,
  confirmation,
  generatedRecoveryPhrase,
  phraseWritten,
  setBusy,
  setMessage,
  setPassphrase,
  setConfirmation,
  setPhraseWritten,
  clearSecrets,
  send,
  commit,
  fail,
}: Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpoint' | 'send' | 'commit'
> & {
  bridge: Bridge;
  agentReady: boolean;
  accountAlias: string;
  passphrase: string;
  confirmation: string;
  generatedRecoveryPhrase: string | null;
  phraseWritten: boolean;
  setBusy: (busy: boolean) => void;
  setMessage: (message: string | null) => void;
  setPassphrase: (phrase: string) => void;
  setConfirmation: (phrase: string) => void;
  setPhraseWritten: (written: boolean) => void;
  clearSecrets: () => void;
  fail: ReturnType<typeof useSetupReconciliation>['fail'];
}) {
  const profile = checkpoint.profile;
  const continueProtection = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !checkpoint.account) return;
    setBusy(true);
    setMessage(null);
    try {
      let next: FirstRunCheckpoint = checkpoint;
      if (passphrase || confirmation) {
        await bridge.setFirstRunPassphrase({
          profile: profile.profile,
          alias: checkpoint.account.alias,
          passphrase,
          confirmation,
        });
        next = transitionFirstRun(next, { type: 'passphrase-set' });
      }
      setPassphrase('');
      setConfirmation('');
      commit(
        transitionFirstRun(next, {
          type: 'navigate',
          state: checkpoint.path === 'invited' ? 'waiting' : 'checklist-own',
        }),
      );
    } catch (error) {
      fail('passphrase', error);
    } finally {
      setBusy(false);
    }
  };

  const finishLocalProtection = async (skip = false): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !checkpoint.account) return;
    if (skip) {
      clearSecrets();
      send({ type: 'finish-local', skipped: true });
      return;
    }
    setBusy(true);
    setMessage(null);
    try {
      let next: FirstRunCheckpoint = checkpoint;
      if (passphrase || confirmation) {
        await bridge.setFirstRunPassphrase({
          profile: profile.profile,
          alias: checkpoint.account.alias,
          passphrase,
          confirmation,
        });
        next = transitionFirstRun(next, { type: 'passphrase-set' });
      }
      if (!next.backupCommitted && generatedRecoveryPhrase && phraseWritten) {
        await bridge.commitOwnerBackup(
          profile.profile,
          accountAlias,
          'paper',
          generatedRecoveryPhrase,
        );
        next = transitionFirstRun(next, { type: 'backup-committed' });
      }
      const skipped = !next.passphraseSet && !next.backupCommitted;
      clearSecrets();
      commit(transitionFirstRun(next, { type: 'finish-local', skipped }));
    } catch (error) {
      fail('passphrase', error);
    } finally {
      setBusy(false);
    }
  };

  const collapseLocalBackup = (): void => {
    setMessage(null);
    send({ type: 'navigate', state: 'protect' });
  };

  const commitBackup = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !generatedRecoveryPhrase || !phraseWritten) return;
    if (checkpoint.backupCommitted) {
      setPhraseWritten(false);
      send({ type: 'navigate', state: 'protect' });
      return;
    }
    setBusy(true);
    try {
      await bridge.commitOwnerBackup(
        profile.profile,
        accountAlias,
        'paper',
        generatedRecoveryPhrase,
      );
      setPhraseWritten(false);
      send({ type: 'backup-committed' });
    } catch (error) {
      fail('backup-commit', error);
    } finally {
      setBusy(false);
    }
  };

  return {
    continueProtection,
    finishLocalProtection,
    collapseLocalBackup,
    commitBackup,
  };
}
