import { normalizeUsername } from '../name-normalization';
import { pastePhrase } from '../phrase-input';
import { fixDeviceName } from '../device-name';
import { LocalCompleteStep } from './first-run-complete-step';
import { ServerVerificationStep } from './first-run-server-step';
import { RecoveryStep } from './first-run-recovery-step';
import {
  SetupSidebar,
  FirstRunAppSidebar,
  AddedDetails,
  Foot,
  Pane,
} from './first-run-view';
export { FirstRunChecklistStatus } from './first-run-view';
import {
  useSetupCheckpoint,
  useSetupSession,
} from './first-run/use-setup-session';
import { useAccountOperations } from './first-run/use-account-operations';
import { ServerAddressStep } from './first-run/server-address-step';
import { AccountSetupStep } from './first-run/account-setup-step';
import { SigninPrimaryAction, SigninSections } from './first-run/signin-step';
import { JoiningChoice } from './first-run/setup-method-choice';
import { WaitingForTeamStep } from './first-run/waiting-for-team-step';
import { SetupChecklistStep } from './first-run/setup-checklist-step';
import { useProtectionActions } from './first-run/use-protection-actions';
import { useRecoveryBackup } from './first-run/use-recovery-backup';
import {
  protectionNavigationPolicy,
  setupNavigationEvent,
  selectedSigninMethod,
  type SigninMethod,
} from './first-run/session-transitions';
import { useSetupReconciliation } from './first-run/use-setup-reconciliation';
import { useServerWorkflow } from './first-run/use-server-workflow';
import { useTeamDiscovery } from './first-run/use-team-discovery';
export { profileNameFor } from './first-run/use-server-workflow';
import { retainSetup, updateRetainedSetup } from '../first-run-recovery';
import { useConcealOnInactive } from '../use-conceal-on-inactive';
import { SsoPanel } from '../components/sso-panel';
import { provisioningInFlight } from '../first-run-operations';
import type { ProvisioningIntent } from '../first-run-state';
import { sharedSetupRead, useSlowSetup } from '../first-run-loading';
import { NavigationPrompt, useNavigationGuard } from '../navigation-guard';
import type { NavigationGuard } from '../location';
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { ReactNode } from 'react';
import {
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  KindIcon,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  Toggle,
} from '../components';
import type { FilterKind } from '../components';
import type {
  Bridge,
  CommandError,
  GoProfileCandidate,
  GoProfileDiscovery,
  PendingOperation,
} from '../bridge';
import { isAgentReadinessError, normalizeCommandError } from '../bridge';
import { transitionFirstRun } from '../first-run-state';
import type {
  FirstRunCheckpoint,
  FirstRunPath,
  FirstRunStateName,
} from '../first-run-state';
import { presentFirstRunFailure } from '../first-run-failure';
import type { RailAgentState } from '../shell/sidebar';
import type { Location } from '../location';
import { displayPath, kindOf, nameOf, storeReadable } from '../model';
import type { AgentSnapshot } from '../model';
import { GoProfileChooser } from './go-profile-chooser';
import { readableBy } from './scope';
import { useToast } from '/kit/toasts';

const MISSING_SERVER_EXPLANATION =
  'The account you were creating could not be found on the server. This may happen because of a restart, server reset, or other error.';

function MissingServerWarning({ action }: { action?: ReactNode }): ReactNode {
  return (
    <div className="alt-path warning" role="alert">
      <div className="t">
        <b>The server saved for this account is missing</b>
        <span>{MISSING_SERVER_EXPLANATION}</span>
      </div>
      {action}
    </div>
  );
}

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

export function accountAliasFor(username: string): string {
  return (normalizeUsername(username.trim()) ?? username)
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+/, '')
    .replace(/-+$/, '')
    .slice(0, 64);
}

/** Suggest a fallback device name based on platform user agent. */
function suggestedDeviceName(): string {
  if (typeof navigator === 'undefined') return 'Mac';
  const source = `${navigator.platform} ${navigator.userAgent}`;
  if (/iPhone/i.test(source)) return 'iPhone';
  if (/iPad/i.test(source)) return 'iPad';
  if (/Mac/i.test(source)) return 'Mac';
  return 'This computer';
}

export interface FirstRunExperienceProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'first-run' }>;
  onNavigate: (location: Location) => void;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  concealSignal: number;
  agentReady: boolean;
  onRetryAgent?: () => Promise<void>;
  onAgentReadinessFailure?: (error: CommandError) => void;
  automaticEntry?: boolean;
  managedProfile?: string;
  /** Connection status indicator in the sidebar footer during first-run setup. */
  agent?: RailAgentState;
  blocked?: boolean;
  /** The rail's width, carried in and out of setup. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /** The Devices tab's dot, as the shell computes it for every other screen. */
  devicesAlert?: { description: string } | null;
}

/**
 * A reported failure: the sentence the agent produced, and behind a
 * disclosure, the raw error chain it kept out of that sentence. Every first-run
 * failure reports through here, so the detail a server check offers is the
 * detail account creation, recovery, pairing and the rest offer too.
 */
function FailureText({
  text,
  reason,
}: {
  text: string;
  reason?: string;
}): ReactNode {
  return (
    <>
      {text}
      {reason ? (
        <Toggle label="Details">
          <pre>{reason}</pre>
        </Toggle>
      ) : null}
    </>
  );
}

export function FirstRunExperience(props: FirstRunExperienceProps): ReactNode {
  const [session, setSession] = useState(0);
  const [sessionEntry, setSessionEntry] = useState<FirstRunCheckpoint | null>(
    null,
  );
  return (
    <FirstRunSession
      key={session}
      sessionEntry={sessionEntry}
      {...props}
      onReplaceSession={(next) => {
        setSessionEntry(next);
        props.onNavigate({
          kind: 'first-run',
          step: next.state,
          path: next.path,
        });
        setSession((value) => value + 1);
      }}
    />
  );
}

function FirstRunSession({
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
  agent = 'ready',
  blocked = false,
  collapsed = false,
  onToggleCollapsed,
  devicesAlert = null,
  onReplaceSession,
  sessionEntry,
}: FirstRunExperienceProps & {
  onReplaceSession: (next: FirstRunCheckpoint) => void;
  sessionEntry: FirstRunCheckpoint | null;
}): ReactNode {
  const toasts = useToast();
  const { checkpoint, checkpointRef, mounted, commit, send } =
    useSetupCheckpoint({
      sessionEntry,
      automaticEntry,
      snapshot,
      bridge,
      location,
      onNavigate,
      agentReady,
    });
  const facts = bridge.firstRunFixture?.[checkpoint.path];
  const [username, setUsername] = useState(
    () => checkpoint.account?.username ?? facts?.username ?? '',
  );
  const [deviceName, setDeviceName] = useState(
    () =>
      checkpoint.account?.deviceName ??
      fixDeviceName(facts?.deviceName ?? suggestedDeviceName()),
  );
  const [email, setEmail] = useState('');
  const [invite, setInvite] = useState('');
  const showSso =
    checkpoint.accountMethod === 'organization' || Boolean(checkpoint.sso);
  const [ssoPrimarySlot, setSsoPrimarySlot] = useState<HTMLElement | null>(
    null,
  );
  const [passphrase, setPassphrase] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [recoveryPhrase, setRecoveryPhrase] = useState('');
  const [pairingPhrase, setPairingPhrase] = useState('');
  // The sign-in method the user picked on the sign-in step. Not persisted:
  // it is a choice within one page, and leaving the sign-in path clears it.
  const [chosenSigninMethod, setChosenSigninMethod] =
    useState<SigninMethod | null>(null);
  const [goDiscovery, setGoDiscovery] = useState<GoProfileDiscovery | null>(
    null,
  );
  const [goCandidate, setGoCandidate] = useState<GoProfileCandidate | null>(
    null,
  );
  const goCandidateExplicit = useRef(false);
  const [goChooserDismissed, setGoChooserDismissed] = useState(false);
  // The fork on "Set up FOKS": use a CLI account or create a new one. The
  // page exists only when candidates were found, so the existing account is
  // the default.
  const [goStart, setGoStart] = useState<'existing' | 'new'>('existing');
  const [goScanError, setGoScanError] = useState<string | null>(null);
  const [goScanAttempt, setGoScanAttempt] = useState(0);
  const {
    generatedRecoveryPhrase,
    recoveryPhraseConcealed,
    setRecoveryPhraseConcealed,
    phraseWritten,
    setPhraseWritten,
    phraseOperation,
    prepare: prepareBackup,
    clearBackup,
    navigateBackup,
  } = useRecoveryBackup();
  // Pending path selection before confirmation.
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [otherMutationBusy, setBusy] = useState(false);
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
  const profile = checkpoint.profile;
  const { pending, lastFailure, fail } = useSetupReconciliation({
    bridge,
    agentReady,
    checkpoint,
    mounted,
    onRefreshSnapshot,
    setMessage,
    setUsername,
  });
  const serverWorkflow = useServerWorkflow({
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
    onAddressEdited: () => {
      if (!goCandidateExplicit.current) setGoCandidate(null);
    },
    setMessage,
    fail,
  });
  const {
    address,
    setAddress,
    addressInvalid,
    serverAddressInput,
    editServerAddress,
    checkServer,
    serverCheckFailure,
    managedStatus,
    managedStatusError,
    setManagedStatusAttempt,
    managedReport,
    selectManagedProfile,
  } = serverWorkflow;
  const teamDiscovery = useTeamDiscovery({
    bridge,
    agentReady,
    checkpoint,
    checkpointRef,
    mounted,
    commit,
    busy: otherMutationBusy || serverWorkflow.busy || phraseOperation !== null,
    onRefreshSnapshot,
    setMessage,
    fail,
  });
  const {
    discover,
    selectDiscoveredGroup,
    discoveredGroups,
    outcome: discoveryOutcome,
  } = teamDiscovery;
  const mutationBusy =
    otherMutationBusy || serverWorkflow.busy || teamDiscovery.busy;
  const busyOperation =
    otherMutationBusy || teamDiscovery.busy
      ? 'other'
      : serverWorkflow.busy
        ? 'server-check'
        : null;
  const busy = mutationBusy || phraseOperation !== null;
  const accountOperations = useAccountOperations({
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
  });
  const {
    identityLoading,
    identityError,
    identityWaiting,
    identityProblem,
    refreshAccountIdentity,
    operationStatus,
    operationProblem,
    operationResumable,
    operationChecked,
    operationRunning,
    existingAccountAdoptable,
    duplicateAlias,
    setDuplicateAlias,
    checkOperationStatus,
    adoptExistingAccount,
    adoptDuplicateAccount,
  } = accountOperations;

  useEffect(() => {
    const hasUserName = Boolean(
      checkpoint.account?.username || facts?.username,
    );
    const hasDeviceName = Boolean(
      checkpoint.account?.deviceName || facts?.deviceName,
    );
    if (hasUserName && hasDeviceName) return;
    const fallback = suggestedDeviceName();
    let alive = true;
    void bridge
      .appInfo()
      .then((info) => {
        if (!alive) return;
        const userName = info.userName?.trim();
        const computerName = info.computerName?.trim();
        if (!hasUserName && userName)
          setUsername((current) => current || userName);
        if (!hasDeviceName && computerName)
          setDeviceName((current) =>
            current === fallback ? fixDeviceName(computerName) : current,
          );
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [
    bridge,
    checkpoint.account?.deviceName,
    checkpoint.account?.username,
    facts?.deviceName,
    facts?.username,
  ]);

  // CLI profiles are discovered on the joining step for the chooser, and again
  // on the account steps when no result is loaded: a resumed checkpoint skips
  // the joining step, and the CLI import and pairing cards depend on a
  // candidate that is not persisted.
  const accountStep =
    checkpoint.state === 'account' || checkpoint.state === 'existing';
  const discoveryWanted =
    checkpoint.state === 'who'
      ? !goChooserDismissed
      : accountStep && goDiscovery === null;
  useEffect(() => {
    if (!agentReady || !bridge.native || !discoveryWanted) return;
    let alive = true;
    setGoScanError(null);
    void sharedSetupRead(bridge, 'cli-discovery', () =>
      bridge.discoverGoProfiles(),
    )
      .then((discovery) => {
        if (alive) setGoDiscovery(discovery);
      })
      .catch((error) => {
        if (!alive) return;
        const typed = normalizeCommandError(error);
        if (isAgentReadinessError(typed) && onAgentReadinessFailure) {
          onAgentReadinessFailure(typed);
          return;
        }
        setGoScanError(typed.message);
      });
    return () => {
      alive = false;
    };
  }, [
    agentReady,
    bridge,
    checkpoint.state,
    discoveryWanted,
    goChooserDismissed,
    goScanAttempt,
    onAgentReadinessFailure,
  ]);

  const state = checkpoint.state;
  // Navigating away from sign-in resets the selected sign-in method. Failed
  // operations transition through `operation-pending` back to this view,
  // preserving the selected method and displaying the error inline.
  useEffect(() => {
    if (state !== 'existing' && state !== 'operation-pending')
      setChosenSigninMethod(null);
  }, [state]);
  const slow = useSlowSetup(
    busy ||
      identityLoading ||
      personalRefreshing ||
      agentRetrying ||
      (state === 'who' &&
        bridge.native &&
        !goChooserDismissed &&
        !goDiscovery &&
        !goScanError) ||
      (state === 'local' && !managedStatus && !managedStatusError),
  );
  /**
   * The raw error chain behind a reported failure, offered only while the
   * text that failure reported is still the text on screen. A later message
   * from anywhere else clears it rather than carrying a stale chain.
   */
  const reasonFor = (shown: string | null): string | undefined => {
    if (!shown || !lastFailure) return undefined;
    const presented = presentFirstRunFailure(lastFailure);
    return shown === lastFailure.error.message || shown === presented.detail
      ? presented.reason
      : undefined;
  };
  const serverCheckPresentation = serverCheckFailure
    ? presentFirstRunFailure(serverCheckFailure)
    : null;
  const accountAlias =
    checkpoint.account?.alias ??
    checkpoint.sso?.alias ??
    facts?.accountAlias ??
    accountAliasFor(username);
  const usernameAliasInvalid = username.trim().length > 0 && !accountAlias;
  const goCandidates = useMemo(
    () =>
      goDiscovery?.candidates.filter(
        (candidate) => candidate.pairable || candidate.copyable,
      ) ?? [],
    [goDiscovery],
  );
  // On "Set up FOKS", a single usable candidate is selected
  // without a click. `goCandidateExplicit` is set when the user picks a
  // candidate or continues with the preselected one.
  const goChooserOpen =
    state === 'who' && !goChooserDismissed && goCandidates.length > 0;
  useEffect(() => {
    if (!goChooserOpen || goCandidate || goCandidates.length !== 1) return;
    setGoCandidate(goCandidates[0]);
  }, [goChooserOpen, goCandidate, goCandidates]);
  // On the account steps, select the CLI profile for the verified server when
  // none was chosen: the alias match wins, otherwise a single profile on that
  // host. Several unrelated profiles on the same host stay unselected rather
  // than importing or pairing the wrong account.
  const profileHostId = profile?.hostId;
  useEffect(() => {
    if (!accountStep || goCandidate || !profileHostId) return;
    const onHost = goCandidates.filter(
      (candidate) => candidate.hostId === profileHostId,
    );
    const byAlias = onHost.filter(
      (candidate) =>
        candidate.username !== undefined &&
        accountAliasFor(candidate.username) === accountAlias,
    );
    const match =
      byAlias.length === 1
        ? byAlias[0]
        : onHost.length === 1
          ? onHost[0]
          : undefined;
    if (!match) return;
    setGoCandidate(match);
    // Mirror the chooser: suggest the CLI username as the account alias.
    if (match.username)
      setUsername(
        (current) => current || accountAliasFor(match.username ?? ''),
      );
  }, [accountStep, goCandidate, goCandidates, profileHostId, accountAlias]);
  const admin = facts?.admin ?? 'team administrator';
  const adminShort = facts?.admin
    ? facts.admin.split('.')[0]
    : 'the team administrator';
  const group = checkpoint.group?.name ?? facts?.groupName ?? 'your team';
  const addedStores = checkpoint.group
    ? snapshot.stores.filter(
        (store) =>
          store.kind === 'team' &&
          store.active &&
          storeReadable(snapshot, store.id) &&
          store.server === profile?.profile &&
          store.account === checkpoint.account?.alias &&
          store.alias === checkpoint.group?.alias &&
          store.team_kind === checkpoint.group?.kind &&
          store.team_id_hex === checkpoint.group?.teamIdHex,
      )
    : [];
  const addedStore = addedStores.length === 1 ? addedStores[0]?.id : undefined;
  const accountStores =
    checkpoint.account && profile
      ? snapshot.accounts.filter(
          (candidate) =>
            candidate.server === profile.profile &&
            candidate.alias === checkpoint.account?.alias,
        )
      : [];
  const accountStore =
    accountStores.length === 1 ? accountStores[0]?.store : undefined;
  const personalAvailable = Boolean(
    accountStore && storeReadable(snapshot, accountStore),
  );
  const accountStoreRecord = accountStore
    ? snapshot.stores.find((candidate) => candidate.id === accountStore)
    : undefined;
  const accountItemCount = accountStore
    ? snapshot.items.filter((item) => item.store === accountStore).length
    : 0;

  const go = useCallback(
    (next: FirstRunStateName): void => {
      const policy = protectionNavigationPolicy(
        { state, backupCommitted: checkpoint.backupCommitted },
        next,
      );
      navigateBackup(policy);
      setMessage(null);
      setDuplicateAlias(null);
      setConnectionErrors({ copy: null, recover: null, pair: null });
      setRecoveryPhrase('');
      setPairingPhrase('');
      setInvite('');
      if (policy.clearPassphrase) {
        setPassphrase('');
        setConfirmation('');
      }
      send(setupNavigationEvent(checkpointRef.current, next));
    },
    [
      checkpoint.backupCommitted,
      checkpointRef,
      send,
      setDuplicateAlias,
      state,
      navigateBackup,
    ],
  );
  const openExisting = useCallback((): void => {
    go('existing');
  }, [go]);
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
    action: import('../sso-contract').SsoAction,
    operation: () => Promise<import('../sso-contract').SsoProgress>,
  ): Promise<import('../sso-contract').SsoProgress> => {
    let progress: import('../sso-contract').SsoProgress | undefined;
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

  /**
   * Whether a secret is visible when the guard is evaluated. `clearSecrets`
   * updates this value before sidebar navigation, so the verdict reflects the
   * cleared state without waiting for another render.
   */
  const secretsHeld = useRef(false);
  secretsHeld.current = Boolean(
    invite ||
    passphrase ||
    confirmation ||
    recoveryPhrase ||
    pairingPhrase ||
    generatedRecoveryPhrase,
  );

  const clearSecrets = useCallback((): void => {
    clearBackup();
    setInvite('');
    setPassphrase('');
    setConfirmation('');
    setRecoveryPhrase('');
    setPairingPhrase('');
    secretsHeld.current = false;
  }, [clearBackup]);

  /**
   * Why the sidebar's Leave setup is inert: a write is out that this screen is
   * the only report of. A provisioning that has been acknowledged is not one
   * of them. That is what Finish later leaves behind.
   */
  const leaveDisabled =
    mutationBusy &&
    busyOperation !== 'server-check' &&
    !checkpoint.provisioning;

  const {
    recoveryActions,
    restartError,
    setRestartError,
    confirmingRestart,
    setConfirmingRestart,
    reenterSetup,
    startSetupOver,
  } = useSetupSession({
    bridge,
    checkpoint,
    checkpointRef,
    mounted,
    mutationBusy,
    operationRunning,
    clearSecrets,
    invalidateIdentity: accountOperations.invalidateIdentity,
    onReplaceSession,
  });

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
      (!secretsHeld.current && !leaveDisabled)
        ? null
        : { verdict: 'refuse', reason: 'Finish or leave setup first.' },
    [leaveDisabled, state],
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

  const {
    continueProtection,
    finishLocalProtection,
    collapseLocalBackup,
    commitBackup,
  } = useProtectionActions({
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

  let content: ReactNode;
  const reviewServerSettings = (
    <Button
      onClick={() =>
        onNavigate({
          kind: 'settings',
          section: 'account',
          profile: profile?.profile,
        })
      }
    >
      Review server settings
    </Button>
  );
  /* The account alias and this device’s name identify the same account
     whichever way it is reached, so both account pages draw the same two
     rows. On the sign-in path the chosen method's own row joins them. */
  const identityRows = (
    <>
      <InsetRow label="Your name">
        <input
          aria-label="Your name"
          value={checkpoint.account?.username ?? username}
          placeholder="yourname"
          disabled={Boolean(checkpoint.account)}
          onChange={(event) => editUsername(event.target.value)}
        />
      </InsetRow>
      <InsetRow label="This device’s name">
        <input
          value={checkpoint.account?.deviceName ?? deviceName}
          placeholder="Your device"
          disabled={Boolean(checkpoint.account)}
          onChange={(event) => setDeviceName(event.target.value)}
          onBlur={() => setDeviceName(fixDeviceName(deviceName))}
        />
      </InsetRow>
    </>
  );
  const aliasInvalidNotice = usernameAliasInvalid ? (
    <p className="crit" role="alert">
      Your name must contain at least one letter or number.
    </p>
  ) : null;
  /* Sign-in authentication methods: CLI pairing and credential import when an
     eligible FOKS CLI profile is detected, followed by recovery phrase recovery.
     When only one method is available, it is selected automatically. */
  const pendingFor = (kind: PendingOperation['kind']): boolean =>
    pending.some((row) => row.kind === kind && row.alias === accountAlias);
  const signinMethod = selectedSigninMethod({
    candidate: goCandidate,
    chosen: chosenSigninMethod,
    pending,
    alias: accountAlias,
  });
  const signinPrimary = (
    <SigninPrimaryAction
      signinMethod={signinMethod}
      busy={busy}
      accountAlias={accountAlias}
      recoveryPhrase={recoveryPhrase}
      pairingPhrase={pairingPhrase}
      deviceName={deviceName}
      recoveryPending={pendingFor('account-recovery')}
      recover={recover}
      copyGoCandidate={copyGoCandidate}
      acceptPairing={acceptPairing}
    />
  );
  const methodError = (key: keyof typeof connectionErrors): ReactNode => {
    const text = connectionErrors[key];
    return text ? (
      <p className="crit" role="alert">
        <FailureText text={text} reason={reasonFor(text)} />
      </p>
    ) : null;
  };
  const signinSections = (first: number): ReactNode => (
    <SigninSections
      first={first}
      signinMethod={signinMethod}
      pairable={Boolean(goCandidate?.pairable)}
      copyable={Boolean(goCandidate?.copyable)}
      accountConnected={Boolean(checkpoint.account)}
      identityRows={identityRows}
      aliasInvalidNotice={aliasInvalidNotice}
      recoveryPhrase={recoveryPhrase}
      pairingPhrase={pairingPhrase}
      busy={busy}
      accountAlias={accountAlias}
      deviceName={deviceName}
      setChosenSigninMethod={setChosenSigninMethod}
      setRecoveryPhrase={setRecoveryPhrase}
      setPairingPhrase={setPairingPhrase}
      acceptPairing={acceptPairing}
      onCopyCommand={(value) =>
        void bridge.copyText(value).then(() => toasts.show('Command copied.'))
      }
      error={
        signinMethod
          ? methodError(signinMethod === 'import' ? 'copy' : signinMethod)
          : null
      }
    />
  );
  /* Where the account page's Back goes; signing in shares it. */
  const accountBackTarget = checkpoint.managedLocal ? 'local' : 'checked';
  if (state === 'operation-pending') {
    const intent = checkpoint.provisioning;
    const settled =
      !operationRunning &&
      !busy &&
      !identityLoading &&
      !(intent && provisioningInFlight(bridge, intent.id));
    const adoptable = existingAccountAdoptable && settled;
    const abortable =
      operationChecked &&
      operationStatus &&
      settled &&
      recoveryActions.canChooseAnotherAccount;
    const runningLabel =
      intent?.kind === 'recovery'
        ? 'Recovering…'
        : intent?.kind === 'pairing'
          ? 'Connecting…'
          : 'Creating…';
    content = (
      <Pane title="Check account setup" header={false}>
        <h1>{busy ? 'Connecting your account' : 'Check account setup'}</h1>
        <p className="lead">
          {busy
            ? 'Account setup is running.'
            : operationProblem !== 'profile-missing' && operationStatus
              ? operationStatus
              : 'Checking account status…'}
        </p>
        {operationProblem === 'profile-missing' && operationStatus ? (
          <MissingServerWarning />
        ) : null}
        {operationResumable && intent?.kind === 'recovery' ? (
          <div className="local-field-card">
            <label className="local-field-row">
              <span>Recovery phrase</span>
              <input
                type="password"
                value={recoveryPhrase}
                onChange={(event) => setRecoveryPhrase(event.target.value)}
                onPaste={(event) => pastePhrase(event, setRecoveryPhrase)}
              />
            </label>
          </div>
        ) : null}
        <div className="setup-actions operation-actions">
          {adoptable ? (
            <Button
              variant="primary"
              onClick={() => void adoptExistingAccount()}
            >
              Continue with existing account
            </Button>
          ) : operationResumable ? (
            <Button
              variant="primary"
              disabled={
                busy || (intent?.kind === 'recovery' && !recoveryPhrase.trim())
              }
              onClick={() => void resumeAccountOperation()}
            >
              Resume account setup
            </Button>
          ) : null}
          <Button
            variant={adoptable || operationResumable ? 'plain' : 'primary'}
            disabled={busy || identityLoading}
            busy={busy || identityLoading}
            onClick={() => void checkOperationStatus()}
          >
            {busy
              ? runningLabel
              : identityLoading
                ? 'Checking status…'
                : 'Check status'}
          </Button>
          {abortable ? (
            <Button onClick={discardProvisioning}>Go back</Button>
          ) : null}
        </div>
        {intent?.kind === 'sso' && intent.ssoOperationId && profile && !busy ? (
          <SsoPanel
            bridge={bridge}
            profile={profile.profile}
            account={intent.alias}
            login={false}
            deviceName={intent.deviceName}
            initialOperationId={intent.ssoOperationId}
            initialHardware={checkpoint.sso?.hardware}
            resumeOnly
            executeSignup={executeSsoSignup}
            onComplete={() => void checkOperationStatus()}
          />
        ) : null}
      </Pane>
    );
  } else if (state === 'identity-pending')
    content = (
      <Pane title="Load account details" header={false}>
        <h1>Your account is connected</h1>
        <p className="lead">
          Your account is connected. Try again to load its details, or finish
          setup later.
        </p>
        {identityProblem === 'profile-missing' ? (
          <MissingServerWarning
            action={
              <Button onClick={discardProvisionedAccount}>
                Set up a different account
              </Button>
            }
          />
        ) : identityError ? (
          <p className="crit" role="alert">
            {identityError}
          </p>
        ) : identityWaiting ? (
          <p className="status" role="status">
            {identityWaiting}
          </p>
        ) : null}
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={identityLoading || !agentReady}
            busy={identityLoading}
            onClick={() => void refreshAccountIdentity()}
          >
            {identityLoading
              ? 'Loading account details…'
              : 'Retry loading account'}
          </Button>
          {reviewServerSettings}
        </div>
        {identityProblem &&
        identityProblem !== 'profile-missing' &&
        !identityLoading ? (
          <div className="alt-path">
            <div className="t">
              <b>Set up a different account</b>
              <span>
                Your existing accounts and server settings will be kept.
              </span>
            </div>
            <Button onClick={discardProvisionedAccount}>
              Set up a different account
            </Button>
          </div>
        ) : null}
      </Pane>
    );
  else if (state === 'local')
    content = (
      <Pane
        title="Set up FOKS"
        header={false}
        foot={
          <Foot>
            <Button
              variant="primary"
              disabled={!managedReport}
              onClick={() => selectManagedProfile()}
            >
              Continue
            </Button>
          </Foot>
        }
      >
        <h1>Set up FOKS</h1>
        <p className="lead">
          A private FOKS server is already running on this device. Use it to
          create your Personal vault.
        </p>
        <div className="local-server-card">
          <div className="local-server-head">
            <span className="local-server-mark" aria-hidden="true">
              <Icon name="server" />
            </span>
            <span className="local-server-title">
              <b>Local server</b>
              <small>On this device</small>
            </span>
            <span className="local-ready">
              <i />
              {managedReport ? 'Ready' : 'Checking'}
            </span>
          </div>
          <dl className="local-server-facts">
            <dt>Address</dt>
            <dd>
              <code>
                {managedStatus?.configuredEndpoint ?? 'localhost:4430'}
              </code>
            </dd>
            <dt>Trust</dt>
            <dd>App-managed certificate</dd>
            <dt>Storage</dt>
            <dd>Local only</dd>
          </dl>
        </div>
        {managedStatusError ? (
          <div className="crit" role="alert">
            <b>Local server unavailable</b>
            {managedStatusError}
            <div className="btns">
              <Button
                onClick={() => setManagedStatusAttempt((value) => value + 1)}
              >
                Check again
              </Button>
            </div>
          </div>
        ) : null}
        <div className="local-alternates">
          <SectionLabel>Other options</SectionLabel>
          <div className="btns">
            <Button onClick={() => send({ type: 'choose', path: 'own' })}>
              Connect to another server…
            </Button>
            <Button
              disabled={!managedReport}
              onClick={() => selectManagedProfile(true)}
            >
              Recover an existing account…
            </Button>
          </div>
        </div>
      </Pane>
    );
  else if (
    state === 'who' &&
    agentReady &&
    bridge.native &&
    !goChooserDismissed &&
    !goDiscovery &&
    !goScanError
  )
    content = (
      <Pane title="Checking this device" header={false}>
        <h1>Looking for existing FOKS accounts</h1>
        <p className="lead">
          Checking for existing accounts from the official FOKS CLI. No changes
          will be made to your data.
        </p>
        <div className="setup-actions">
          <Button variant="primary" disabled busy>
            Checking…
          </Button>
          <Button onClick={() => setGoChooserDismissed(true)}>
            Skip for now
          </Button>
        </div>
      </Pane>
    );
  else if (goChooserOpen)
    content = (
      <Pane title="Set up FOKS" header={false} wide>
        <h1>Set up FOKS</h1>
        <p className="lead">
          A FOKS CLI account was found on this Mac. You can use it here or set
          up a new one.
        </p>
        <Inset>
          <RadioGroup label="How to start">
            <div className="choice">
              <RadioCard
                title="Use an existing account"
                detail="Continue with an account already active in the CLI."
                selected={goStart === 'existing'}
                onSelect={() => setGoStart('existing')}
              />
              {goStart === 'existing' ? (
                <div className="choice-body">
                  <GoProfileChooser
                    nested
                    candidates={goCandidates}
                    selected={goCandidate?.candidateId ?? null}
                    onSelect={(candidate) => {
                      goCandidateExplicit.current = true;
                      setGoCandidate(candidate);
                      setGoStart('existing');
                    }}
                  />
                </div>
              ) : null}
            </div>
            <div className="choice">
              <RadioCard
                title="Create a new account"
                detail="Generate a new cryptographic identity and vault account."
                selected={goStart === 'new'}
                onSelect={() => setGoStart('new')}
              />
            </div>
          </RadioGroup>
        </Inset>
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={goStart === 'existing' && !goCandidate}
            onClick={() => {
              if (goStart === 'new') {
                goCandidateExplicit.current = false;
                setGoCandidate(null);
                setGoChooserDismissed(true);
                return;
              }
              if (!goCandidate) return;
              // Continuing confirms a preselected candidate as well, so the
              // server step keeps it when the address is edited.
              goCandidateExplicit.current = true;
              setAddress(goCandidate.serverHint ?? '');
              setUsername(
                goCandidate.username
                  ?.toLowerCase()
                  .replace(/[^a-z0-9_-]+/g, '-') ?? 'personal',
              );
              send({ type: 'choose', path: 'own', returning: true });
            }}
          >
            Continue
          </Button>
        </div>
      </Pane>
    );
  else if (state === 'who' || state === 'boot')
    content = (
      <Pane title="How are you joining?" header={false} wide>
        <h1>How are you joining?</h1>
        <p className="lead">
          Choose an option to get started. You will set up your account and
          server in the next steps.
        </p>
        {goScanError ? (
          <div className="crit" role="alert">
            Could not check existing CLI profiles: {goScanError}
            <Button
              onClick={() => {
                setGoDiscovery(null);
                setGoChooserDismissed(false);
                setGoScanAttempt((value) => value + 1);
              }}
            >
              Check again
            </Button>
          </div>
        ) : null}
        {message ? (
          <div className="crit" role="alert">
            <p>Setup could not be completed: {message}</p>
            <Button
              size="sm"
              onClick={() => {
                setMessage(null);
              }}
            >
              Try again
            </Button>
          </div>
        ) : null}
        <JoiningChoice value={pendingPath} onChange={setPendingPath} />
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={!pendingPath || !agentReady || busy}
            onClick={() => {
              if (pendingPath && agentReady)
                send({ type: 'choose', path: pendingPath });
            }}
          >
            {!agentReady ? 'Preparing service…' : 'Continue'}
          </Button>
          {goChooserDismissed && goCandidates.length > 0 ? (
            <Button onClick={() => setGoChooserDismissed(false)}>Back</Button>
          ) : null}
        </div>
      </Pane>
    );
  else if (['address', 'no-address', 'error'].includes(state))
    content = (
      <ServerAddressStep
        state={state}
        path={checkpoint.path}
        busy={busy}
        adminShort={adminShort}
        address={address}
        addressInvalid={addressInvalid}
        inputRef={serverAddressInput}
        serverCheckPresentation={serverCheckPresentation}
        message={message}
        go={go}
        checkServer={checkServer}
        editServerAddress={editServerAddress}
        onCopy={(value) =>
          void bridge
            .copyText(value)
            .then(() => toasts.show('Copied to clipboard.'))
        }
      />
    );
  else if ((state === 'checked' || state === 'compare') && profile)
    content = (
      <ServerVerificationStep
        checkpoint={checkpoint}
        state={state}
        profile={profile}
        address={address}
        busy={busy}
        inputRef={serverAddressInput}
        onBack={() => go('address')}
        onContinue={() =>
          checkpoint.returning ? openExisting() : go('account')
        }
        onCheck={() => void checkServer()}
        onAddressChange={editServerAddress}
      />
    );
  else if (state === 'account' && checkpoint.managedLocal)
    content = (
      <Pane
        title="Your account"
        header={false}
        foot={
          <Foot back={() => go('local')}>
            <Button
              variant="primary"
              disabled={
                busy ||
                usernameAliasInvalid ||
                (!checkpoint.account &&
                  (!username.trim() ||
                    !fixDeviceName(deviceName) ||
                    !accountAlias))
              }
              onClick={() =>
                checkpoint.account ? go('protect') : void createAccount()
              }
            >
              {checkpoint.account
                ? 'Continue'
                : pending.some(
                      (row) =>
                        row.kind === 'account-signup' &&
                        row.alias === accountAlias,
                    )
                  ? 'Resume account setup'
                  : 'Create my account'}
            </Button>
          </Foot>
        }
      >
        <h1>Create your account</h1>
        <p className="lead">
          Choose a username for this server and a name for this device.
        </p>
        <div className="local-field-card">
          <label className="local-field-row">
            <span>Username</span>
            <input
              value={username}
              placeholder="yourname"
              autoFocus
              disabled={Boolean(checkpoint.account)}
              onChange={(event) => editUsername(event.target.value)}
            />
          </label>
          <label className="local-field-row">
            <span>Device name</span>
            <input
              value={deviceName}
              placeholder="Your device"
              disabled={Boolean(checkpoint.account)}
              onChange={(event) => setDeviceName(event.target.value)}
              onBlur={() => setDeviceName(fixDeviceName(deviceName))}
            />
          </label>
        </div>
        <details className="local-more">
          <summary>More options</summary>
          <label className="local-field-row">
            <span>Email (optional)</span>
            <input
              value={email}
              disabled={Boolean(checkpoint.account)}
              placeholder="you@example.net"
              onChange={(event) => setEmail(event.target.value)}
            />
          </label>
          <label className="local-field-row">
            <span>Invite code (optional)</span>
            <input
              value={invite}
              disabled={Boolean(checkpoint.account)}
              onChange={(event) => setInvite(event.target.value)}
            />
          </label>
        </details>
        {usernameAliasInvalid ? (
          <p className="crit" role="alert">
            Username must contain at least one letter or number.
          </p>
        ) : null}
        {message ? (
          <p className="crit" role="alert">
            <FailureText text={message} reason={reasonFor(message)} />
          </p>
        ) : null}
        {duplicateAlias && !busy && !identityLoading ? (
          <div className="band info" role="status">
            <span className="t">
              “{duplicateAlias.alias}” is already set up on this device for this
              server. Use that account, or choose a different username.
            </span>
            <span className="a">
              <Button
                variant="primary"
                onClick={() => void adoptDuplicateAccount()}
              >
                Use existing account
              </Button>
            </span>
          </div>
        ) : null}
        <button
          className="lnk local-recover-link"
          onClick={() => openExisting()}
        >
          Recover an existing account…
        </button>
      </Pane>
    );
  else if (
    state === 'account' ||
    (state === 'existing' && !checkpoint.managedLocal)
  ) {
    // Both choices render on this one page. `existing` is the same page with
    // "Sign in to an existing account" selected and the sign-in method group
    // drawn as the next section.
    const signingIn = state === 'existing';
    const cliAccountSelected = signingIn && goCandidateExplicit.current;
    // Organization sign-up is a third choice in the same radio group. It is
    // offered only while the server is known and no account exists yet, and
    // its own panel carries the primary action.
    const ssoAvailable = Boolean(profile) && !checkpoint.account;
    const ssoSelected = !signingIn && ssoAvailable && showSso;
    const organizationSignup = profile ? (
      <SsoPanel
        key={`${profile.profile}/${accountAlias}`}
        embedded
        primarySlot={ssoPrimarySlot}
        bridge={bridge}
        profile={profile.profile}
        account={accountAlias}
        login={false}
        deviceName={fixDeviceName(deviceName)}
        invite={invite}
        disabled={busy}
        initialOperationId={
          checkpoint.sso?.alias === accountAlias
            ? checkpoint.sso.operationId
            : undefined
        }
        initialHardware={checkpoint.sso?.hardware}
        executeSignup={executeSsoSignup}
        onProgress={(progress, hardware) => {
          if (
            ['cancelled', 'expired', 'denied', 'rejected'].includes(
              progress.state,
            ) &&
            !checkpointRef.current.provisioning
          ) {
            const next = {
              ...checkpointRef.current,
              sso: undefined,
              accountMethod: 'create' as const,
            };
            updateRetainedSetup(checkpointRef.current, next);
            commit(next);
            return;
          }
          if (
            progress.operationId &&
            !checkpointRef.current.provisioning &&
            !checkpointRef.current.provisionedAccount
          )
            commit({
              ...checkpointRef.current,
              sso: {
                operationId: progress.operationId,
                alias: accountAlias,
                hardware,
              },
            });
        }}
        onComplete={() => {
          setInvite('');
          accountOperations.accountProvisioned(
            accountAlias,
            fixDeviceName(deviceName),
          );
        }}
      />
    ) : null;
    content = (
      <AccountSetupStep
        checkpoint={checkpoint}
        signingIn={signingIn}
        cliAccountSelected={cliAccountSelected}
        ssoAvailable={ssoAvailable}
        ssoSelected={ssoSelected}
        busy={busy}
        usernameAliasInvalid={usernameAliasInvalid}
        username={username}
        deviceName={deviceName}
        signupPending={pending.some(
          (row) => row.kind === 'account-signup' && row.alias === accountAlias,
        )}
        email={email}
        invite={invite}
        goCandidatePresent={Boolean(goCandidate)}
        accountBackTarget={accountBackTarget}
        identityLoading={identityLoading}
        duplicateAlias={duplicateAlias}
        message={message}
        identityRows={identityRows}
        aliasInvalidNotice={aliasInvalidNotice}
        signinPrimary={signinPrimary}
        signinSections={signinSections}
        organizationSignup={organizationSignup}
        failureText={
          <FailureText text={message ?? ''} reason={reasonFor(message)} />
        }
        go={go}
        createAccount={createAccount}
        openExisting={openExisting}
        setEmail={setEmail}
        setInvite={setInvite}
        onPrimarySlot={setSsoPrimarySlot}
        onChooseOrganization={() => {
          clearSecrets();
          send({ type: 'select-account-method', method: 'organization' });
        }}
        adoptDuplicateAccount={adoptDuplicateAccount}
      />
    );
  }
  // Only the managed-local path keeps a separate page for an existing account;
  // its Create your account pane has no setup-method radios, so the sign-in
  // method is this page's first section.
  else if (state === 'existing')
    content = (
      <Pane
        title="Your account"
        header={false}
        scope="Device authorization required"
        wide
        foot={
          <Foot
            back={() => go(checkpoint.account ? 'protect' : accountBackTarget)}
          >
            {checkpoint.account ? (
              <Button onClick={() => go('protect')}>Skip to latest step</Button>
            ) : (
              <>
                <Button
                  onClick={() => {
                    go('account');
                  }}
                >
                  Create a new account
                </Button>
                {signinPrimary}
              </>
            )}
          </Foot>
        }
      >
        <h1>Add this device to your account</h1>
        <p className="lead">
          Your account already exists on {profile?.canonicalName}. Recover it
          with your recovery phrase
          {goCandidate ? ' or connect using the official FOKS CLI' : ''}.
        </p>
        {signinSections(1)}
      </Pane>
    );
  else if (state === 'protect' || state === 'phrase')
    content = (
      <RecoveryStep
        state={state}
        checkpoint={checkpoint}
        busy={busy}
        recoveryPhrase={generatedRecoveryPhrase}
        phraseConcealed={recoveryPhraseConcealed}
        revealPhrase={() => setRecoveryPhraseConcealed(false)}
        phraseWritten={phraseWritten}
        setPhraseWritten={setPhraseWritten}
        go={go}
        finishLocalProtection={finishLocalProtection}
        collapseLocalBackup={collapseLocalBackup}
        message={message}
        passphrase={passphrase}
        confirmation={confirmation}
        setPassphrase={setPassphrase}
        setConfirmation={setConfirmation}
        continueProtection={continueProtection}
        mutationBusy={mutationBusy}
        commitBackup={commitBackup}
      />
    );
  else if (state === 'local-done')
    content = (
      <LocalCompleteStep
        personalAvailable={personalAvailable}
        accountStoreRecord={accountStoreRecord}
        accountStore={accountStore}
        personalRefreshing={personalRefreshing}
        retryPersonal={retryPersonal}
        onNavigate={onNavigate}
        profile={profile}
        snapshot={snapshot}
        accountItemCount={accountItemCount}
        personalRefreshError={personalRefreshError}
      />
    );
  else if (state === 'waiting')
    content = (
      <WaitingForTeamStep
        checkpoint={checkpoint}
        busy={busy}
        message={message}
        admin={admin}
        adminShort={adminShort}
        group={group}
        discoveredGroups={discoveredGroups}
        discoveryOutcome={discoveryOutcome}
        discover={discover}
        selectDiscoveredGroup={selectDiscoveredGroup}
        go={go}
        onChooseAnotherTeam={() => {
          commit({
            ...checkpoint,
            selectedGroup: undefined,
            group: undefined,
            added: false,
          });
          setMessage(null);
        }}
        onCopy={(value) =>
          void bridge
            .copyText(value)
            .then(() => toasts.show('Copied to clipboard.'))
        }
      />
    );
  else if (state === 'checklist-invited' || state === 'checklist-own')
    content = (
      <SetupChecklistStep
        checkpoint={checkpoint}
        personalRefreshError={personalRefreshError}
        admin={admin}
        adminShort={adminShort}
        group={group}
        accountStore={accountStore}
        personalRefreshing={personalRefreshing}
        retryPersonal={retryPersonal}
        onNavigate={onNavigate}
        go={go}
        onCopy={(value) =>
          void bridge.copyText(value).then(() => toasts.show('Message copied.'))
        }
      />
    );
  else
    content = (
      <Pane title={group} subtitle={`Team on ${profile?.canonicalName}`} wide>
        <Notice
          // The pane's title already names the team, so the notice states the
          // condition alone, in the same words the store access takeover uses.
          severity={addedStore ? 'info' : 'warn'}
          title={addedStore ? `Joined ${group}` : 'Vault unavailable'}
          actions={
            <>
              <Button onClick={() => onNavigate({ kind: 'all' })}>
                Dismiss
              </Button>
              {!addedStore ? (
                <Button
                  disabled={personalRefreshing}
                  onClick={() => {
                    setPersonalRefreshing(true);
                    void onRefreshSnapshot(true)
                      .catch((error) =>
                        setMessage(normalizeCommandError(error).message),
                      )
                      .finally(() => setPersonalRefreshing(false));
                  }}
                >
                  Retry loading team
                </Button>
              ) : null}
              <Button
                variant="primary"
                disabled={!addedStore}
                onClick={() => {
                  if (addedStore)
                    onNavigate({ kind: 'store', ref: addedStore });
                }}
              >
                Open team
              </Button>
            </>
          }
        >
          <p>
            You have been added to this team as{' '}
            <code>{checkpoint.account?.username}</code>. You can now access the
            items listed below when the vault is available.
          </p>
          {message ? <p role="status">{message}</p> : null}
        </Notice>
        <div className="hdr">
          <span />
          <span>Name</span>
          <span>Access</span>
          <span>Version</span>
        </div>
        {snapshot.items
          .filter((item) => item.store === addedStore)
          .slice(0, 4)
          .map((item) => (
            <div className="row" key={`${item.store}|${item.path}`}>
              <KindIcon kind={kindOf(item) as FilterKind} />
              <span className="name">
                {nameOf(item.path)}
                <small>{displayPath(item.path)}</small>
              </span>
              <span>
                <Chip>{readableBy(snapshot, item).label}</Chip>
              </span>
              <span className="n">v{item.version}</span>
            </div>
          ))}
      </Pane>
    );

  const appMode = ['added', 'checklist-invited', 'checklist-own'].includes(
    state,
  );
  const readinessBlocker =
    !agentReady && onRetryAgent ? (
      <Pane title="Service setup" header={false}>
        <h1>Finish preparing this device</h1>
        <p className="lead">
          FOKS needs to finish preparing and checking local setup before account
          setup can continue. Your progress is still saved.
        </p>
        {agentRetryError ? (
          <div className="crit" role="alert">
            <b>Setup could not be completed</b>
            {agentRetryError}
          </div>
        ) : null}
        <div className="setup-actions">
          <Button
            variant="primary"
            disabled={agentRetrying}
            onClick={() => {
              setAgentRetrying(true);
              setAgentRetryError(null);
              void onRetryAgent()
                .catch((error) => {
                  setAgentRetryError(normalizeCommandError(error).message);
                })
                .finally(() => setAgentRetrying(false));
            }}
          >
            {agentRetrying ? 'Preparing service…' : 'Retry setup'}
          </Button>
        </div>
      </Pane>
    ) : null;
  return (
    <>
      {appMode ? (
        <FirstRunAppSidebar
          snapshot={snapshot}
          native={Boolean(bridge.native)}
          checkpoint={checkpoint}
          groupName={checkpoint.group?.name ?? group}
          location={location}
          onNavigate={onNavigate}
          onReenter={reenterSetup}
          collapsed={collapsed}
          onToggleCollapsed={onToggleCollapsed}
          agent={agent}
          blocked={blocked}
          devicesAlert={devicesAlert}
        />
      ) : (
        <SetupSidebar
          checkpoint={checkpoint}
          native={Boolean(bridge.native)}
          agent={agent}
          blocked={blocked}
          pendingPath={pendingPath}
          onAnotherServer={
            !busy &&
            checkpoint.managedLocal &&
            !checkpoint.account &&
            !checkpoint.provisioning &&
            !checkpoint.provisionedAccount
              ? () => send({ type: 'choose', path: 'own' })
              : undefined
          }
          onRecoverAccount={
            !busy &&
            checkpoint.managedLocal &&
            !checkpoint.account &&
            !checkpoint.provisioning &&
            !checkpoint.provisionedAccount
              ? () => selectManagedProfile(true)
              : undefined
          }
          recoverEnabled={Boolean(managedReport)}
          cancelDisabled={leaveDisabled}
          onRestart={() => setConfirmingRestart(true)}
          restartDisabled={!recoveryActions.canRestart}
          restartReason={recoveryActions.restartReason}
          onCancel={() => {
            // Clear entered values before navigating so the guard allows the
            // exit.
            clearSecrets();
            if (state === 'phrase')
              send({ type: 'navigate', state: 'protect' });
            onNavigate({ kind: 'all' });
          }}
        />
      )}
      <main className="main first-run-main" aria-busy={agentReady && busy}>
        {restartError ? (
          <p role="alert" className="crit">
            {restartError}
          </p>
        ) : null}
        {slow || phraseOperation ? (
          <div className="bandstrip">
            <div className="band" role="status">
              <span className="t">
                {slow
                  ? `This is taking longer than expected. ${
                      checkpoint.provisioning
                        ? 'You can finish later while account setup completes.'
                        : 'FOKS is still waiting for a response.'
                    }`
                  : 'Preparing your recovery phrase…'}
              </span>
              {phraseOperation ? (
                <span className="a">
                  <Button
                    onClick={() => {
                      clearSecrets();
                      go('protect');
                    }}
                  >
                    Back to recovery options
                  </Button>
                </span>
              ) : null}
            </div>
          </div>
        ) : null}
        <div
          data-setup-controls=""
          style={{ display: 'contents' }}
          inert={agentReady && busy && !checkpoint.provisioning}
        >
          {readinessBlocker ?? content}
        </div>
      </main>
      {state === 'added' ? (
        <AddedDetails snapshot={snapshot} storeId={addedStore} />
      ) : null}
      {confirmingRestart ? (
        <NavigationPrompt
          verdict={{
            verdict: 'prompt',
            title: 'Restart setup?',
            body: 'The setup in progress on this device will be discarded. If an account was already created on the server, you will need a recovery key to reconnect to it.',
            confirm: 'Start over',
          }}
          onConfirm={startSetupOver}
          onCancel={() => setConfirmingRestart(false)}
        />
      ) : null}
    </>
  );
}
