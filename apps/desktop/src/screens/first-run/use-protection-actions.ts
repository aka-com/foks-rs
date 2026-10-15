import { transitionFirstRun } from '../../first-run-state';
import type { FirstRunCheckpoint } from '../../first-run-state';
import type { SetupCore } from './setup-core';
import type { SetupSecrets } from './use-setup-secrets';

/** Account protection commands: UI drafts are ephemeral; only receipts enter the checkpoint. */
export function useProtectionActions({
  core,
  accountAlias,
  secrets,
}: {
  core: SetupCore;
  accountAlias: string;
  secrets: SetupSecrets;
}) {
  const {
    bridge,
    agentReady,
    checkpoint,
    send,
    commit,
    setMessage,
    fail,
    setAccountBusy,
  } = core;
  const { passphrase, confirmation, setPassphrase, setConfirmation } = secrets;
  const { generatedRecoveryPhrase, phraseWritten, setPhraseWritten } =
    secrets.backup;
  const clearSecrets = secrets.clear;
  const profile = checkpoint.profile;
  const continueProtection = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !checkpoint.account) return;
    setAccountBusy(true);
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
      setAccountBusy(false);
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
    setAccountBusy(true);
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
      setAccountBusy(false);
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
    setAccountBusy(true);
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
      setAccountBusy(false);
    }
  };

  return {
    continueProtection,
    finishLocalProtection,
    collapseLocalBackup,
    commitBackup,
  };
}
