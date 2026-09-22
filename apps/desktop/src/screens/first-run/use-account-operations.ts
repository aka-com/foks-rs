import { useEffect, useRef, useState } from 'react';
import {
  enqueueProfileWork,
  isAgentReadinessError,
  normalizeCommandError,
} from '../../bridge';
import type { Bridge, CommandError } from '../../bridge';
import {
  provisionedIdentityProblem,
  identityProblemText,
  type IdentityProblem,
} from '../../first-run-identity';
import {
  executeProvisioning,
  provisioningInFlight,
  persistFirstRun,
} from '../../first-run-operations';
import { retainedSetups, sameSetupTarget } from '../../first-run-recovery';
import {
  transitionFirstRun,
  type FirstRunCheckpoint,
  type ProvisioningIntent,
} from '../../first-run-state';
import type { AgentSnapshot } from '../../model';
import type { useFirstRunController } from '../../use-first-run-controller';
import { useAccountIdentity } from './use-account-identity';

export interface AccountOperationInput {
  deviceName: string;
  username: string;
  email: string;
  invite: string;
  phrase: string;
}

export function useAccountOperations({
  bridge,
  agentReady,
  onRefreshSnapshot,
  onAgentReadinessFailure,
  checkpoint,
  checkpointRef,
  mounted,
  commit,
  busy,
  setBusy,
  setMessage,
  setConnectionErrors,
  onReplaceSession,
}: Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpoint' | 'checkpointRef' | 'mounted' | 'commit'
> & {
  bridge: Bridge;
  agentReady: boolean;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  onAgentReadinessFailure?: (error: CommandError) => void;
  busy: boolean;
  setBusy: (value: boolean) => void;
  setMessage: (message: string | null) => void;
  setConnectionErrors: (
    errors: Record<'copy' | 'recover' | 'pair', string | null>,
  ) => void;
  onReplaceSession: (next: FirstRunCheckpoint) => void;
}) {
  const identity = useAccountIdentity({
    bridge,
    agentReady,
    onRefreshSnapshot,
    onAgentReadinessFailure,
    checkpointRef,
    mounted,
    commit,
  });
  const {
    identityLoading,
    setIdentityLoading,
    autoProbedKey,
    refreshAccountIdentity,
  } = identity;
  const [operationStatus, setOperationStatus] = useState<string | null>(null);
  const [operationProblem, setOperationProblem] =
    useState<IdentityProblem | null>(null);
  const [operationResumable, setOperationResumable] = useState(false);
  const [operationChecked, setOperationChecked] = useState(false);
  const [operationRunning, setOperationRunning] = useState(false);
  const [existingAccountAdoptable, setExistingAccountAdoptable] =
    useState(false);
  const [duplicateAlias, setDuplicateAlias] = useState<{
    alias: string;
    deviceName: string;
  } | null>(null);
  const state = checkpoint.state;

  const accountProvisioned = (alias: string, deviceName: string): void => {
    const saved = transitionFirstRun(checkpointRef.current, {
      type: 'account-provisioned',
      alias,
      deviceName: deviceName.trim(),
    });
    // Persist the acknowledgment before any fallible inventory read.
    commit(saved);
    void refreshAccountIdentity(saved);
  };

  const confirmDuplicateAlias = async (
    saved: FirstRunCheckpoint,
    intent: ProvisioningIntent,
  ): Promise<void> => {
    if (!saved.profile) return;
    try {
      const refreshed = await onRefreshSnapshot(true);
      if (!mounted.current || checkpointRef.current.provisioning) return;
      const probe = {
        ...saved,
        provisioning: undefined,
        provisionedAccount: {
          alias: intent.alias,
          deviceName: intent.deviceName,
        },
      };
      if (provisionedIdentityProblem(refreshed, probe)) return;
      setDuplicateAlias({
        alias: intent.alias,
        deviceName: intent.deviceName,
      });
    } catch {
      setDuplicateAlias(null);
    }
  };

  const runAccountOperation = async (
    input: AccountOperationInput,
    kind: ProvisioningIntent['kind'],
    alias: string,
    operation: () => Promise<unknown>,
    extra: Partial<
      Pick<ProvisioningIntent, 'candidateId' | 'ssoOperationId'>
    > = {},
    details: { resume?: boolean; phrase?: string } = {},
  ): Promise<void> => {
    const { deviceName, username, email, invite } = input;
    const current = checkpointRef.current;
    // A new wizard must explicitly reconcile a retained attempt on this target.
    if (!current.provisioning) {
      const retained = retainedSetups().find((entry) =>
        sameSetupTarget(entry.checkpoint, {
          ...current,
          provisionedAccount: { alias, deviceName: deviceName.trim() },
        }),
      );
      if (retained) {
        persistFirstRun(retained.checkpoint);
        mounted.current = false;
        onReplaceSession(retained.checkpoint);
        return;
      }
    }
    const saved: FirstRunCheckpoint = current.provisioning
      ? current
      : {
          ...current,
          state: 'operation-pending',
          provisioning: {
            id: crypto.randomUUID(),
            kind,
            alias,
            deviceName: deviceName.trim(),
            back: state === 'existing' ? 'existing' : 'account',
            ...extra,
          },
        };
    setBusy(true);
    setOperationResumable(false);
    setOperationStatus(null);
    setDuplicateAlias(null);
    try {
      // Dispatch saves the intent synchronously before calling the bridge.
      persistFirstRun(saved);
      const intent = saved.provisioning!;
      const tracked =
        bridge.runFirstRunAccountOperation && intent.kind !== 'sso'
          ? () =>
              bridge.runFirstRunAccountOperation!({
                attempt: {
                  id: intent.id,
                  kind: intent.kind as
                    'signup' | 'recovery' | 'copy' | 'pairing',
                  alias: intent.alias,
                  deviceName: intent.deviceName,
                  ...(intent.candidateId
                    ? { candidateId: intent.candidateId }
                    : {}),
                  profile: saved.profile!.profile,
                  hostId: saved.profile!.hostId,
                },
                resume:
                  Boolean(current.provisioning) || Boolean(details.resume),
                deviceName: intent.deviceName,
                ...(intent.kind === 'signup'
                  ? { username: username.trim(), email, invite }
                  : {}),
                ...(intent.kind === 'recovery' || intent.kind === 'pairing'
                  ? { phrase: details.phrase ?? input.phrase }
                  : {}),
                ...(intent.candidateId
                  ? { candidateId: intent.candidateId }
                  : {}),
              })
          : operation;
      const task = executeProvisioning(
        bridge,
        saved,
        tracked,
        Boolean(current.provisioning) || Boolean(details.resume),
      );
      commit(saved);
      const result = await task;
      if (
        !mounted.current ||
        (checkpointRef.current.provisioning &&
          checkpointRef.current.provisioning.id !== saved.provisioning?.id)
      )
        return;
      commit(result.checkpoint);
      if (result.error) {
        const typed = normalizeCommandError(result.error);
        const detail = typed.message;
        setOperationStatus(detail);
        setMessage(detail);
        setConnectionErrors({
          copy: kind === 'copy' ? detail : null,
          recover: kind === 'recovery' ? detail : null,
          pair: kind === 'pairing' ? detail : null,
        });
        if (result.checkpoint.provisioning)
          void checkOperationStatus(result.checkpoint);
        else if (kind === 'signup' && typed.code === 'already-exists')
          await confirmDuplicateAlias(result.checkpoint, intent);
      } else void refreshAccountIdentity(result.checkpoint);
    } catch (error) {
      if (mounted.current) {
        setOperationStatus(normalizeCommandError(error).message);
        setMessage(normalizeCommandError(error).message);
      }
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const checkOperationStatus = async (
    saved: FirstRunCheckpoint = checkpointRef.current,
  ): Promise<void> => {
    const intent = saved.provisioning;
    if (!intent || !saved.profile || !agentReady || identityLoading) return;
    const profileName = saved.profile.profile;
    autoProbedKey.current = `operation:${intent.id}`;
    setOperationProblem(null);
    if (provisioningInFlight(bridge, intent.id)) {
      setOperationRunning(true);
      setOperationStatus(
        'Account setup is still running. You can finish later while it completes.',
      );
      return;
    }
    setIdentityLoading(true);
    setOperationResumable(false);
    setOperationRunning(false);
    setExistingAccountAdoptable(false);
    let statusError: string | null = null;
    const withStatusError = (text: string): string =>
      statusError ? `${text} (Details: ${statusError})` : text;
    try {
      if (intent.kind !== 'sso' && bridge.firstRunOperationStatus) {
        let outcome: 'complete' | 'rejected' | 'unknown' | 'running' =
          'unknown';
        try {
          outcome = await bridge.firstRunOperationStatus({
            id: intent.id,
            kind: intent.kind,
            alias: intent.alias,
            deviceName: intent.deviceName,
            ...(intent.candidateId ? { candidateId: intent.candidateId } : {}),
            profile: profileName,
            hostId: saved.profile.hostId,
          });
        } catch (error) {
          const typed = normalizeCommandError(error);
          if (isAgentReadinessError(typed)) {
            if (mounted.current) setOperationStatus(typed.message);
            onAgentReadinessFailure?.(typed);
            return;
          }
          statusError = typed.message;
        }
        if (
          !mounted.current ||
          checkpointRef.current.provisioning?.id !== intent.id
        )
          return;
        if (outcome === 'running') {
          setOperationRunning(true);
          setOperationStatus(
            'Account setup is still running. You can finish later while it completes.',
          );
          return;
        }
        if (outcome === 'complete') {
          const acknowledged = transitionFirstRun(
            { ...saved, provisioning: undefined },
            {
              type: 'account-provisioned',
              alias: intent.alias,
              deviceName: intent.deviceName,
            },
          );
          persistFirstRun(acknowledged);
          commit(acknowledged);
          await refreshAccountIdentity(acknowledged);
          return;
        }
        if (outcome === 'rejected') {
          const rejected = {
            ...saved,
            provisioning: undefined,
            state: intent.back,
          };
          persistFirstRun(rejected);
          commit(rejected);
          setMessage(
            'Account setup was not accepted. Review the details and try again.',
          );
          return;
        }
      }
      const refreshed = await onRefreshSnapshot(true);
      const probe = {
        ...saved,
        provisioning: undefined,
        provisionedAccount: {
          alias: intent.alias,
          deviceName: intent.deviceName,
        },
      };
      if (
        !mounted.current ||
        checkpointRef.current.provisioning?.id !== intent.id
      )
        return;
      const problem = provisionedIdentityProblem(refreshed, probe);
      if (problem && problem !== 'account-missing') {
        setOperationProblem(problem);
        setOperationStatus(withStatusError(identityProblemText[problem]));
        return;
      }
      if (intent.kind === 'sso' && intent.ssoOperationId) {
        const progress = await bridge.sso(profileName, intent.alias, {
          action: 'status',
          operation_id: intent.ssoOperationId,
        });
        if (
          !mounted.current ||
          checkpointRef.current.provisioning?.id !== intent.id
        )
          return;
        if (
          progress.operationId === intent.ssoOperationId &&
          progress.accountAlias === intent.alias &&
          progress.purpose === 'signup' &&
          ['complete', 'service-unavailable'].includes(progress.state)
        ) {
          const acknowledged = transitionFirstRun(
            { ...saved, provisioning: undefined },
            {
              type: 'account-provisioned',
              alias: intent.alias,
              deviceName: intent.deviceName,
            },
          );
          persistFirstRun(acknowledged);
          commit(acknowledged);
          await refreshAccountIdentity(acknowledged);
          return;
        }
      }
      const rows = await enqueueProfileWork(bridge, profileName, () =>
        bridge.listPendingOperations(profileName),
      );
      if (
        !mounted.current ||
        checkpointRef.current.provisioning?.id !== intent.id
      )
        return;
      const kind =
        intent.kind === 'signup'
          ? 'account-signup'
          : intent.kind === 'recovery'
            ? 'account-recovery'
            : intent.kind === 'pairing'
              ? 'pairing-acceptance'
              : null;
      const resumable =
        kind !== null &&
        rows.some(
          (row) =>
            row.kind === kind && row.alias === intent.alias && !row.target,
        );
      const adoptable = !resumable && !problem;
      setOperationResumable(resumable);
      setExistingAccountAdoptable(adoptable);
      setOperationStatus(
        withStatusError(
          resumable
            ? 'Account setup was interrupted. Resume it to continue.'
            : adoptable
              ? 'An account with this username already exists on this server. You can use this account, start over, or check your server settings.'
              : 'Account setup still could not be confirmed. Start over, or check your server settings.',
        ),
      );
    } catch (error) {
      if (mounted.current)
        setOperationStatus(normalizeCommandError(error).message);
    } finally {
      if (mounted.current) {
        setIdentityLoading(false);
        setOperationChecked(true);
      }
    }
  };

  const intentId = checkpoint.provisioning?.id;
  useEffect(() => {
    setOperationChecked(false);
    setOperationProblem(null);
    setOperationRunning(false);
    setExistingAccountAdoptable(false);
  }, [intentId]);

  const probeKey =
    state === 'operation-pending'
      ? `operation:${checkpoint.provisioning?.id ?? ''}`
      : state === 'identity-pending'
        ? `identity:${checkpoint.provisionedAccount?.alias ?? ''}`
        : null;
  const automaticProbe = useRef<() => void>(() => {});
  automaticProbe.current = () => {
    if (state === 'operation-pending') void checkOperationStatus();
    else void refreshAccountIdentity();
  };
  useEffect(() => {
    if (!agentReady || !probeKey || busy || identityLoading) return;
    if (autoProbedKey.current === probeKey) return;
    autoProbedKey.current = probeKey;
    if (mounted.current) automaticProbe.current();
  }, [agentReady, autoProbedKey, busy, identityLoading, mounted, probeKey]);

  const acknowledgeExistingAccount = async (
    alias: string,
    deviceName: string,
  ): Promise<void> => {
    const acknowledged = transitionFirstRun(
      { ...checkpointRef.current, provisioning: undefined },
      { type: 'account-provisioned', alias, deviceName },
    );
    persistFirstRun(acknowledged);
    commit(acknowledged);
    setOperationStatus(null);
    setExistingAccountAdoptable(false);
    setDuplicateAlias(null);
    await refreshAccountIdentity(acknowledged);
  };

  const adoptExistingAccount = async (): Promise<void> => {
    const intent = checkpointRef.current.provisioning;
    if (!intent || !existingAccountAdoptable || busy || identityLoading) return;
    await acknowledgeExistingAccount(intent.alias, intent.deviceName);
  };

  const adoptDuplicateAccount = async (): Promise<void> => {
    if (!duplicateAlias || busy || identityLoading) return;
    await acknowledgeExistingAccount(
      duplicateAlias.alias,
      duplicateAlias.deviceName,
    );
  };

  const resumeAccountOperation = async (
    input: AccountOperationInput,
  ): Promise<boolean> => {
    const saved = checkpointRef.current;
    const intent = saved.provisioning;
    if (!intent || !saved.profile || !operationResumable || busy) return false;
    const name = saved.profile.profile;
    await runAccountOperation(input, intent.kind, intent.alias, () => {
      if (intent.kind === 'signup')
        return bridge.resumeFirstRunAccount(name, intent.alias);
      if (intent.kind === 'recovery')
        return bridge.resumeOwnerRecovery(
          name,
          intent.alias,
          input.phrase,
          intent.deviceName,
        );
      if (intent.kind === 'pairing' && intent.candidateId)
        return bridge.resumeGoProfilePairing(
          intent.candidateId,
          name,
          intent.alias,
        );
      throw new Error(
        'This operation cannot be resumed here. Review account settings.',
      );
    });
    return true;
  };

  return {
    identityLoading,
    identityError: identity.identityError,
    identityWaiting: identity.identityWaiting,
    identityProblem: identity.identityProblem,
    invalidateIdentity: identity.invalidateIdentity,
    clearIdentityProblem: identity.clearIdentityProblem,
    refreshAccountIdentity,
    operationStatus,
    operationProblem,
    operationResumable,
    operationChecked,
    operationRunning,
    existingAccountAdoptable,
    duplicateAlias,
    setDuplicateAlias,
    accountProvisioned,
    runAccountOperation,
    resumeAccountOperation,
    checkOperationStatus,
    adoptExistingAccount,
    adoptDuplicateAccount,
    clearOperationProblem: () => {
      setOperationStatus(null);
      setOperationProblem(null);
      setOperationResumable(false);
    },
  };
}
