import { useCallback, useEffect, useState } from 'react';
import { isAgentReadinessError, normalizeCommandError } from '../../bridge';
import { presentFirstRunFailure } from '../../first-run-failure';
import { useSlowSetup } from '../../first-run-loading';
import type { FirstRunPath, FirstRunStateName } from '../../first-run-state';
import {
  protectionNavigationPolicy,
  setupNavigationEvent,
  type SigninMethod,
} from './session-transitions';
import { setupBusy, setupLeaveBlocked, setupMutating } from './setup-busy';
import type { FirstRunSessionProps, SetupSession } from './setup-context';
import type { SetupCore } from './setup-core';
import { setupStores } from './setup-stores';
import { useAccountActions } from './use-account-actions';
import { useAccountOperations } from './use-account-operations';
import { useCliDiscovery } from './use-cli-discovery';
import { useProtectionActions } from './use-protection-actions';
import { useServerWorkflow } from './use-server-workflow';
import { useSetupCheckpoint } from './use-setup-checkpoint';
import { useSetupGuards } from './use-setup-guards';
import { useSetupIdentity } from './use-setup-identity';
import { useSetupReconciliation } from './use-setup-reconciliation';
import { useSetupRestart } from './use-setup-restart';
import { useSetupSecrets } from './use-setup-secrets';
import { useTeamDiscovery } from './use-team-discovery';

/**
 * Builds the setup session FirstRunSession provides to its steps: the
 * checkpoint, the workflows that advance it, the drafts and secrets typed along
 * the way, and the effects that guard them.
 */
export function useFirstRunSession({
  snapshot,
  bridge,
  location,
  onNavigate,
  onRefreshSnapshot,
  concealSignal,
  agentReady,
  onRetryAgent,
  onAgentReadinessFailure,
  automaticEntry = false,
  managedProfile,
  onReplaceSession,
  sessionEntry,
}: FirstRunSessionProps): SetupSession {
  const controller = useSetupCheckpoint({
    sessionEntry,
    automaticEntry,
    snapshot,
    bridge,
    location,
    onNavigate,
    agentReady,
  });
  const { checkpoint, checkpointRef, mounted, commit, send } = controller;
  const state = checkpoint.state;
  const profile = checkpoint.profile;
  const facts = bridge.firstRunFixture?.[checkpoint.path];
  const identity = useSetupIdentity({ bridge, checkpoint });
  const { setUsername, accountAlias } = identity;
  const secrets = useSetupSecrets();
  const { clear: clearSecrets, navigate: navigateSecrets } = secrets;
  const {
    generatedRecoveryPhrase,
    phraseOperation,
    prepare: prepareBackup,
  } = secrets.backup;
  const [ssoPrimarySlot, setSsoPrimarySlot] = useState<HTMLElement | null>(
    null,
  );
  const [chosenSigninMethod, setChosenSigninMethod] =
    useState<SigninMethod | null>(null);
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [accountBusy, setAccountBusy] = useState(false);
  const [personalRefreshing, setPersonalRefreshing] = useState(false);
  const [personalRefreshError, setPersonalRefreshError] = useState<
    string | null
  >(null);
  const [agentRetrying, setAgentRetrying] = useState(false);
  const [agentRetryError, setAgentRetryError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const base = {
    checkpoint,
    checkpointRef,
    mounted,
    commit,
    send,
    bridge,
    agentReady,
    onRefreshSnapshot,
    setMessage,
    setAccountBusy,
  };
  const { pending, lastFailure, fail } = useSetupReconciliation({
    core: base,
    setUsername,
  });
  const core: SetupCore = { ...base, fail };
  const cli = useCliDiscovery({
    bridge,
    agentReady,
    state,
    profileHostId: profile?.hostId,
    accountAlias,
    onAgentReadinessFailure,
    setUsername,
  });
  const goCandidate = cli.candidate;
  const server = useServerWorkflow({
    core,
    snapshot,
    managedProfile,
    goCandidate,
    onAddressEdited: cli.addressEdited,
  });
  // Discovery waits for the other operations; its own flag is not among them.
  const team = useTeamDiscovery({
    core,
    busy: accountBusy || server.busy || phraseOperation !== null,
  });
  const busy = setupBusy({
    account: accountBusy,
    discovery: team.busy,
    serverCheck: server.busy,
    phrase: phraseOperation !== null,
  });
  const mutationBusy = setupMutating(busy);
  const accountOperations = useAccountOperations({
    core,
    busy: busy !== null,
    onAgentReadinessFailure,
    onReplaceSession,
  });
  const { identityLoading, setDuplicateAlias, clearConnectionErrors } =
    accountOperations;

  // Navigating away from sign-in resets the selected sign-in method. Failed
  // operations transition through `operation-pending` back to this view,
  // preserving the selected method and displaying the error inline.
  useEffect(() => {
    if (state !== 'existing' && state !== 'operation-pending')
      setChosenSigninMethod(null);
  }, [state]);
  const slow = useSlowSetup(
    busy !== null ||
      identityLoading ||
      personalRefreshing ||
      agentRetrying ||
      cli.scanPending ||
      (state === 'local' &&
        !server.managedStatus &&
        !server.managedStatusError),
  );
  const reasonFor = (shown: string | null): string | undefined => {
    if (!shown || !lastFailure) return undefined;
    const presented = presentFirstRunFailure(lastFailure);
    return shown === lastFailure.error.message || shown === presented.detail
      ? presented.reason
      : undefined;
  };

  const go = useCallback(
    (next: FirstRunStateName): void => {
      const policy = protectionNavigationPolicy(
        { state, backupCommitted: checkpoint.backupCommitted },
        next,
      );
      navigateSecrets(policy);
      setMessage(null);
      setDuplicateAlias(null);
      clearConnectionErrors();
      send(setupNavigationEvent(checkpointRef.current, next));
    },
    [
      checkpoint.backupCommitted,
      checkpointRef,
      clearConnectionErrors,
      send,
      setDuplicateAlias,
      state,
      navigateSecrets,
    ],
  );
  const restart = useSetupRestart({
    core,
    mutationBusy,
    operationRunning: accountOperations.operationRunning,
    clearSecrets,
    invalidateIdentity: accountOperations.invalidateIdentity,
    onReplaceSession,
  });
  const accountActions = useAccountActions({
    core,
    operations: accountOperations,
    identity,
    secrets,
    candidate: goCandidate,
    pending,
    setRestartError: restart.setRestartError,
  });

  const leaveBlocked = setupLeaveBlocked(
    busy,
    Boolean(checkpoint.provisioning),
  );

  useSetupGuards({
    state,
    agentReady,
    concealSignal,
    secrets,
    leaveBlocked,
    go,
  });

  useEffect(() => {
    if (
      !agentReady ||
      state !== 'phrase' ||
      generatedRecoveryPhrase ||
      !profile ||
      !accountAlias
    )
      return;
    return prepareBackup(bridge, profile.profile, accountAlias, (error) => {
      go('protect');
      fail('backup-prepare', error);
    });
  }, [
    accountAlias,
    agentReady,
    generatedRecoveryPhrase,
    prepareBackup,
    bridge,
    fail,
    go,
    profile,
    state,
  ]);

  const editUsername = (value: string): void => {
    setUsername(value);
    setDuplicateAlias(null);
  };

  const protection = useProtectionActions({ core, accountAlias, secrets });

  const retryPersonal = (): void => {
    setPersonalRefreshing(true);
    setPersonalRefreshError(null);
    void onRefreshSnapshot()
      .catch((error) => {
        const typed = normalizeCommandError(error);
        setPersonalRefreshError(typed.message);
        if (isAgentReadinessError(typed)) onAgentReadinessFailure?.(typed);
      })
      .finally(() => setPersonalRefreshing(false));
  };
  const retryTeam = (): void => {
    setPersonalRefreshing(true);
    void onRefreshSnapshot(true)
      .catch((error) => setMessage(normalizeCommandError(error).message))
      .finally(() => setPersonalRefreshing(false));
  };
  const retryAgent = onRetryAgent
    ? (): void => {
        setAgentRetrying(true);
        setAgentRetryError(null);
        void onRetryAgent()
          .catch((error) => {
            setAgentRetryError(normalizeCommandError(error).message);
          })
          .finally(() => setAgentRetrying(false));
      }
    : null;

  return {
    ...controller,
    bridge,
    snapshot,
    agentReady,
    onNavigate,
    busy,
    slow,
    leaveBlocked,
    message,
    setMessage,
    reasonFor,
    go,
    pending,
    identity: { ...identity, edit: editUsername },
    secrets,
    cli,
    server,
    team,
    account: { ...accountOperations, ...accountActions },
    protection,
    restart,
    stores: setupStores(snapshot, checkpoint, facts),
    personal: {
      refreshing: personalRefreshing,
      error: personalRefreshError,
      retry: retryPersonal,
      retryTeam,
    },
    agentRetry: {
      retrying: agentRetrying,
      error: agentRetryError,
      retry: retryAgent,
    },
    joining: { path: pendingPath, setPath: setPendingPath },
    signin: { chosen: chosenSigninMethod, setChosen: setChosenSigninMethod },
    sso: {
      shown:
        checkpoint.accountMethod === 'organization' || Boolean(checkpoint.sso),
      primarySlot: ssoPrimarySlot,
      setPrimarySlot: setSsoPrimarySlot,
    },
  };
}
