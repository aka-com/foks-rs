import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';
import { isAgentReadinessError, normalizeCommandError } from '../../bridge';
import { fixDeviceName } from '../../device-name';
import { presentFirstRunFailure } from '../../first-run-failure';
import { useSlowSetup } from '../../first-run-loading';
import { retainSetup } from '../../first-run-recovery';
import {
  transitionFirstRun,
  type FirstRunPath,
  type FirstRunStateName,
  type ProvisioningIntent,
} from '../../first-run-state';
import type { NavigationGuard } from '../../location';
import { useNavigationGuard } from '../../navigation-guard';
import type { SsoAction, SsoProgress } from '../../sso-contract';
import { useConcealOnInactive } from '../../use-conceal-on-inactive';
import {
  protectionNavigationPolicy,
  setupNavigationEvent,
  type SigninMethod,
} from './session-transitions';
import { setupBusy, setupLeaveBlocked, setupMutating } from './setup-busy';
import type { FirstRunSessionProps, SetupSession } from './setup-context';
import { setupStores } from './setup-stores';
import { useAccountOperations } from './use-account-operations';
import { useCliDiscovery } from './use-cli-discovery';
import { useProtectionActions } from './use-protection-actions';
import { useServerWorkflow } from './use-server-workflow';
import { useSetupIdentity } from './use-setup-identity';
import { useSetupReconciliation } from './use-setup-reconciliation';
import { useSetupSecrets } from './use-setup-secrets';
import { useSetupCheckpoint, useSetupRestart } from './use-setup-session';
import { useTeamDiscovery } from './use-team-discovery';

/**
 * Terminal first-run states where account setup is complete, all user input is
 * committed, and navigation to the main application is active. These states do
 * not prompt for confirmation when navigating away.
 */
const FIRST_RUN_SETTLED: readonly FirstRunStateName[] = [
  'added',
  'local-done',
  'checklist-invited',
  'checklist-own',
];

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
  const { username, setUsername, deviceName, email, accountAlias } = identity;
  const secrets = useSetupSecrets();
  const {
    invite,
    setInvite,
    recoveryPhrase,
    setRecoveryPhrase,
    pairingPhrase,
    setPairingPhrase,
    clear: clearSecrets,
    navigate: navigateSecrets,
  } = secrets;
  const {
    generatedRecoveryPhrase,
    setRecoveryPhraseConcealed,
    phraseOperation,
    prepare: prepareBackup,
  } = secrets.backup;
  const [ssoPrimarySlot, setSsoPrimarySlot] = useState<HTMLElement | null>(
    null,
  );
  const [chosenSigninMethod, setChosenSigninMethod] =
    useState<SigninMethod | null>(null);
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [accountBusy, setBusy] = useState(false);
  const [personalRefreshing, setPersonalRefreshing] = useState(false);
  const [personalRefreshError, setPersonalRefreshError] = useState<
    string | null
  >(null);
  const [agentRetrying, setAgentRetrying] = useState(false);
  const [agentRetryError, setAgentRetryError] = useState<string | null>(null);
  const [connectionErrors, setConnectionErrors] = useState<
    Record<'copy' | 'recover' | 'pair', string | null>
  >({ copy: null, recover: null, pair: null });
  const [message, setMessage] = useState<string | null>(null);
  const { pending, lastFailure, fail } = useSetupReconciliation({
    bridge,
    agentReady,
    checkpoint,
    mounted,
    onRefreshSnapshot,
    setMessage,
    setUsername,
  });
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
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    mounted,
    send,
    snapshot,
    managedProfile,
    onRefreshSnapshot,
    goCandidate,
    onAddressEdited: cli.addressEdited,
    setMessage,
    fail,
  });
  const team = useTeamDiscovery({
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    mounted,
    commit,
    busy:
      setupBusy({
        account: accountBusy,
        discovery: false,
        serverCheck: server.busy,
        phrase: phraseOperation !== null,
      }) !== null,
    onRefreshSnapshot,
    setMessage,
    fail,
  });
  const busy = setupBusy({
    account: accountBusy,
    discovery: team.busy,
    serverCheck: server.busy,
    phrase: phraseOperation !== null,
  });
  const mutationBusy = setupMutating(busy);
  const accountOperations = useAccountOperations({
    bridge,
    agentReady,
    onRefreshSnapshot,
    onAgentReadinessFailure,
    checkpoint,
    checkpointRef,
    mounted,
    commit,
    busy: busy !== null,
    setBusy,
    setMessage,
    setConnectionErrors,
    onReplaceSession,
  });
  const { identityLoading, setDuplicateAlias } = accountOperations;

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
      setConnectionErrors({ copy: null, recover: null, pair: null });
      send(setupNavigationEvent(checkpointRef.current, next));
    },
    [
      checkpoint.backupCommitted,
      checkpointRef,
      send,
      setDuplicateAlias,
      state,
      navigateSecrets,
    ],
  );
  const runAccountOperation = (
    kind: ProvisioningIntent['kind'],
    alias: string,
    operation: () => Promise<unknown>,
    extra: Partial<
      Pick<ProvisioningIntent, 'candidateId' | 'ssoOperationId'>
    > = {},
    details: { resume?: boolean; phrase?: string } = {},
  ): Promise<void> =>
    accountOperations.runAccountOperation(
      {
        deviceName: fixDeviceName(deviceName),
        username,
        email,
        invite,
        phrase: recoveryPhrase,
      },
      kind,
      alias,
      operation,
      extra,
      details,
    );

  const restart = useSetupRestart({
    bridge,
    checkpoint,
    checkpointRef,
    mounted,
    mutationBusy,
    operationRunning: accountOperations.operationRunning,
    clearSecrets,
    invalidateIdentity: accountOperations.invalidateIdentity,
    onReplaceSession,
  });
  const { setRestartError } = restart;

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
    accountOperations.clearOperationProblem();
    setMessage(null);
    setConnectionErrors({ copy: null, recover: null, pair: null });
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
    accountOperations.clearIdentityProblem();
  };

  const resumeAccountOperation = async (): Promise<void> => {
    const resumed = await accountOperations.resumeAccountOperation({
      deviceName: fixDeviceName(deviceName),
      username,
      email,
      invite,
      phrase: recoveryPhrase,
    });
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

  const leaveBlocked = setupLeaveBlocked(
    busy,
    Boolean(checkpoint.provisioning),
  );

  /**
   * Setup is left through the sidebar, which clears what was typed on the way
   * out. The rail and Control-Tab are not on screen until the end steps, but
   * ⌘K and the back swipe are, and neither of them clears anything: a step
   * holding a secret or an unfinished write answers for them here. A move
   * within setup is the step machine publishing its own address, and the end
   * steps are past everything this protects.
   */
  const setupGuard = useCallback<NavigationGuard>(
    (intent) =>
      (intent.kind === 'navigate' && intent.location.kind === 'first-run') ||
      FIRST_RUN_SETTLED.includes(state) ||
      (!secrets.held.current && !leaveBlocked)
        ? null
        : { verdict: 'refuse', reason: 'Finish or leave setup first.' },
    [leaveBlocked, secrets.held, state],
  );
  useNavigationGuard(setupGuard);

  useLayoutEffect(() => {
    if (agentReady) return;
    clearSecrets();
    if (state === 'phrase') go('protect');
  }, [agentReady, clearSecrets, go, state]);

  const lastConcealSignal = useRef(concealSignal);
  useLayoutEffect(() => {
    if (concealSignal === lastConcealSignal.current) return;
    lastConcealSignal.current = concealSignal;
    clearSecrets();
    if (state === 'phrase') go('protect');
  }, [clearSecrets, concealSignal, go, state]);

  useConcealOnInactive(
    () => setRecoveryPhraseConcealed(true),
    state === 'phrase',
  );

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

  const createAccount = async (): Promise<void> => {
    if (!agentReady) return;
    if (
      !profile ||
      !username.trim() ||
      !fixDeviceName(deviceName) ||
      !accountAlias
    )
      return;
    setBusy(true);
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
      setBusy(false);
    }
  };

  const copyGoCandidate = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !goCandidate?.copyable || !accountAlias) return;
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, copy: null }));
    try {
      await runAccountOperation(
        'copy',
        accountAlias,
        async () => {
          const copied = await bridge.copyGoProfileDevice(
            goCandidate.candidateId,
            profile.profile,
            accountAlias,
          );
          if (copied.alias !== accountAlias)
            throw new Error(
              'The imported profile belongs to a different account.',
            );
        },
        { candidateId: goCandidate.candidateId },
      );
    } catch (error) {
      fail('account-copy', error, (message) =>
        setConnectionErrors((old) => ({ ...old, copy: message })),
      );
    } finally {
      setBusy(false);
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
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, recover: null }));
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
        setConnectionErrors((old) => ({ ...old, recover: message })),
      );
    } finally {
      setBusy(false);
    }
  };

  const acceptPairing = async (resume: boolean): Promise<void> => {
    if (!agentReady) return;
    if (
      !profile ||
      !goCandidate?.pairable ||
      !accountAlias ||
      !fixDeviceName(deviceName)
    )
      return;
    const phrase = pairingPhrase;
    if (!resume && !phrase.trim()) return;
    setPairingPhrase('');
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, pair: null }));
    try {
      await runAccountOperation(
        'pairing',
        accountAlias,
        async () => {
          const provision = resume
            ? await bridge.resumeGoProfilePairing(
                goCandidate.candidateId,
                profile.profile,
                accountAlias,
              )
            : await bridge.acceptGoProfilePairing(
                goCandidate.candidateId,
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
        { candidateId: goCandidate.candidateId },
        { resume, phrase },
      );
    } catch (error) {
      fail('account-pairing', error, (message) =>
        setConnectionErrors((old) => ({ ...old, pair: message })),
      );
    } finally {
      setBusy(false);
    }
  };

  const protection = useProtectionActions({
    bridge,
    agentReady,
    checkpoint,
    accountAlias,
    secrets,
    setBusy,
    setMessage,
    send,
    commit,
    fail,
  });

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
    onRefreshSnapshot,
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
    account: {
      ...accountOperations,
      connectionErrors,
      create: createAccount,
      copyCandidate: copyGoCandidate,
      recover,
      acceptPairing,
      resume: resumeAccountOperation,
      discardProvisioning,
      discardProvisionedAccount,
      executeSsoSignup,
    },
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
