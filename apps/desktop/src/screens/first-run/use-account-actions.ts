import type { GoProfileCandidate, PendingOperation } from '../../bridge';
import { normalizeCommandError } from '../../bridge';
import { fixDeviceName } from '../../device-name';
import { retainSetup } from '../../first-run-recovery';
import {
  transitionFirstRun,
  type ProvisioningIntent,
} from '../../first-run-state';
import type { SsoAction, SsoProgress } from '../../sso-contract';
import type { SetupCore } from './setup-core';
import type {
  AccountOperationInput,
  useAccountOperations,
} from './use-account-operations';
import type { SetupIdentity } from './use-setup-identity';
import type { SetupSecrets } from './use-setup-secrets';

/** The account commands the setup steps start: one per way of getting an account onto this device. */
export interface AccountActions {
  /** Signs up, or resumes the agent's interrupted signup for the same alias. */
  create: () => Promise<void>;
  /** Imports the chosen FOKS CLI profile's device credentials. */
  copyCandidate: () => Promise<void>;
  /** Recovers the account with the typed recovery phrase. */
  recover: () => Promise<void>;
  /** Accepts, or with `resume` resumes, pairing with a FOKS CLI device. */
  acceptPairing: (resume: boolean) => Promise<void>;
  /** Resumes the interrupted provisioning the checkpoint records. */
  resume: () => Promise<void>;
  /** Drops the provisioning record, keeping the attempt as a retained setup. */
  discardProvisioning: () => void;
  /** Drops the provisioned account, keeping the attempt as a retained setup. */
  discardProvisionedAccount: () => void;
  /** Runs an organization sign-up as a provisioning operation and returns its final progress. */
  executeSsoSignup: (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ) => Promise<SsoProgress>;
}

/**
 * The account commands, each run through useAccountOperations with the
 * drafts and secrets typed on the account step. A failure is reported beside
 * the sign-in method that failed, or as the session message.
 */
export function useAccountActions({
  core,
  operations,
  identity,
  secrets,
  candidate,
  pending,
  setRestartError,
}: {
  core: SetupCore;
  operations: ReturnType<typeof useAccountOperations>;
  identity: SetupIdentity;
  secrets: SetupSecrets;
  /** The FOKS CLI profile picked for import or pairing, if any. */
  candidate: GoProfileCandidate | null;
  pending: readonly PendingOperation[];
  setRestartError: (message: string | null) => void;
}): AccountActions {
  const {
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    commit,
    setMessage,
    fail,
    setAccountBusy,
  } = core;
  const profile = checkpoint.profile;
  const { username, deviceName, email, accountAlias } = identity;
  const {
    invite,
    setInvite,
    recoveryPhrase,
    setRecoveryPhrase,
    pairingPhrase,
    setPairingPhrase,
  } = secrets;
  const { reportConnectionError, clearConnectionErrors } = operations;

  const input = (): AccountOperationInput => ({
    deviceName: fixDeviceName(deviceName),
    username,
    email,
    invite,
    phrase: recoveryPhrase,
  });
  const runAccountOperation = (
    kind: ProvisioningIntent['kind'],
    alias: string,
    operation: () => Promise<unknown>,
    extra: Partial<
      Pick<ProvisioningIntent, 'candidateId' | 'ssoOperationId'>
    > = {},
    details: { resume?: boolean; phrase?: string } = {},
  ): Promise<void> =>
    operations.runAccountOperation(
      input(),
      kind,
      alias,
      operation,
      extra,
      details,
    );

  const discardProvisioning = (): void => {
    try {
      retainSetup(checkpointRef.current);
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
      return;
    }
    commit(
      transitionFirstRun(checkpointRef.current, {
        type: 'discard-provisioning',
      }),
    );
    operations.clearOperationProblem();
    setMessage(null);
    clearConnectionErrors();
  };

  const discardProvisionedAccount = (): void => {
    try {
      retainSetup(checkpointRef.current);
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
      return;
    }
    commit(
      transitionFirstRun(checkpointRef.current, {
        type: 'discard-provisioned-account',
      }),
    );
    operations.clearIdentityProblem();
  };

  const resume = async (): Promise<void> => {
    const resumed = await operations.resumeAccountOperation(input());
    if (resumed) setRecoveryPhrase('');
  };

  const executeSsoSignup = async (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ): Promise<SsoProgress> => {
    let progress: SsoProgress | undefined;
    await runAccountOperation(
      'sso',
      accountAlias,
      async () => {
        progress = await operation();
        if (
          progress.accountAlias !== accountAlias ||
          !('operation_id' in action) ||
          progress.operationId !== action.operation_id ||
          progress.purpose !== 'signup' ||
          !['complete', 'service-unavailable'].includes(progress.state)
        )
          throw {
            code: 'ambiguous',
            message: 'Check sign-in status to continue.',
            ambiguous: true,
            retryable: false,
            fatal: false,
          };
      },
      {
        ssoOperationId:
          'operation_id' in action ? action.operation_id : undefined,
      },
    );
    if (!progress) throw new Error('Check sign-in status to continue.');
    return progress;
  };

  const create = async (): Promise<void> => {
    if (!agentReady) return;
    if (
      !profile ||
      !username.trim() ||
      !fixDeviceName(deviceName) ||
      !accountAlias
    )
      return;
    setAccountBusy(true);
    setMessage(null);
    try {
      const resumable = pending.find(
        (row) =>
          row.kind === 'account-signup' &&
          row.alias === accountAlias &&
          !row.target,
      );
      await runAccountOperation(
        'signup',
        accountAlias,
        async () => {
          if (resumable)
            await bridge.resumeFirstRunAccount(profile.profile, accountAlias);
          else
            await bridge.createFirstRunAccount({
              profile: profile.profile,
              alias: accountAlias,
              username: username.trim(),
              deviceName: fixDeviceName(deviceName),
              email,
              invite,
            });
        },
        {},
        { resume: Boolean(resumable) },
      );
      setInvite('');
    } catch (error) {
      fail('account-signup', error);
    } finally {
      setAccountBusy(false);
    }
  };

  const copyCandidate = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !candidate?.copyable || !accountAlias) return;
    setAccountBusy(true);
    reportConnectionError('copy', null);
    try {
      await runAccountOperation(
        'copy',
        accountAlias,
        async () => {
          const copied = await bridge.copyGoProfileDevice(
            candidate.candidateId,
            profile.profile,
            accountAlias,
          );
          if (copied.alias !== accountAlias)
            throw new Error(
              'The imported profile belongs to a different account.',
            );
        },
        { candidateId: candidate.candidateId },
      );
    } catch (error) {
      fail('account-copy', error, (message) =>
        reportConnectionError('copy', message),
      );
    } finally {
      setAccountBusy(false);
    }
  };

  const recover = async (): Promise<void> => {
    if (!agentReady) return;
    if (
      !profile ||
      !recoveryPhrase.trim() ||
      !fixDeviceName(deviceName) ||
      !accountAlias
    )
      return;
    setAccountBusy(true);
    reportConnectionError('recover', null);
    try {
      await runAccountOperation(
        'recovery',
        accountAlias,
        async () => {
          const resumable = pending.find(
            (row) =>
              row.kind === 'account-recovery' &&
              row.alias === accountAlias &&
              !row.target,
          );
          if (resumable)
            await bridge.resumeOwnerRecovery(
              profile.profile,
              accountAlias,
              recoveryPhrase,
              fixDeviceName(deviceName),
            );
          else
            await bridge.recoverOwnerAccount(
              profile.profile,
              accountAlias,
              recoveryPhrase,
              fixDeviceName(deviceName),
            );
        },
        {},
        {
          resume: pending.some(
            (row) =>
              row.kind === 'account-recovery' &&
              row.alias === accountAlias &&
              !row.target,
          ),
          phrase: recoveryPhrase,
        },
      );
      setRecoveryPhrase('');
    } catch (error) {
      fail('account-recovery', error, (message) =>
        reportConnectionError('recover', message),
      );
    } finally {
      setAccountBusy(false);
    }
  };

  const acceptPairing = async (resume: boolean): Promise<void> => {
    if (!agentReady) return;
    if (
      !profile ||
      !candidate?.pairable ||
      !accountAlias ||
      !fixDeviceName(deviceName)
    )
      return;
    const phrase = pairingPhrase;
    if (!resume && !phrase.trim()) return;
    setPairingPhrase('');
    setAccountBusy(true);
    reportConnectionError('pair', null);
    try {
      await runAccountOperation(
        'pairing',
        accountAlias,
        async () => {
          const provision = resume
            ? await bridge.resumeGoProfilePairing(
                candidate.candidateId,
                profile.profile,
                accountAlias,
              )
            : await bridge.acceptGoProfilePairing(
                candidate.candidateId,
                profile.profile,
                accountAlias,
                fixDeviceName(deviceName),
                phrase,
              );
          if (provision.alias !== accountAlias) {
            throw new Error(
              'The paired device credentials belong to a different account.',
            );
          }
        },
        { candidateId: candidate.candidateId },
        { resume, phrase },
      );
    } catch (error) {
      fail('account-pairing', error, (message) =>
        reportConnectionError('pair', message),
      );
    } finally {
      setAccountBusy(false);
    }
  };

  return {
    create,
    copyCandidate,
    recover,
    acceptPairing,
    resume,
    discardProvisioning,
    discardProvisionedAccount,
    executeSsoSignup,
  };
}
