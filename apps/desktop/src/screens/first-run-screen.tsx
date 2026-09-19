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
import { useServerWorkflow } from './first-run/use-server-workflow';
import { useTeamDiscovery } from './first-run/use-team-discovery';
export { profileNameFor } from './first-run/use-server-workflow';
import { retainSetup, updateRetainedSetup } from '../first-run-recovery';
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
  Band,
  Button,
  Chip,
  CopyBox,
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
import {
  enqueueProfileWork,
  isAgentReadinessError,
  normalizeCommandError,
} from '../bridge';
import {
  completedFirstRunSteps,
  firstRunStepCount,
  transitionFirstRun,
  setupBackTarget,
} from '../first-run-state';
import type {
  FirstRunCheckpoint,
  FirstRunPath,
  FirstRunStateName,
} from '../first-run-state';
import {
  classifyFirstRunFailure,
  presentFirstRunFailure,
  reconcileFirstRunFailure,
  type FirstRunFailure,
  type FirstRunOperation,
} from '../first-run-failure';
import type { FoksIconName } from '../icons';
import type { RailAgentState } from '../shell/sidebar';
import type { Location } from '../location';
import { kindOf, storeReadable } from '../model';
import type { AgentSnapshot } from '../model';
import { GoProfileChooser } from './go-profile-chooser';
import { readableBy } from './scope';
import { useToast } from '/kit/toasts';

const PERSONAL_FIXED =
  'Your Personal vault is private to your account. To share items with others, use a team.';

const MISSING_SERVER_EXPLANATION =
  'The account you were creating could not be found on the server. This may happen because of a restart, server reset, or other error.';

/** Authentication methods for an existing account. */
type SigninMethod = 'recover' | 'import' | 'pair';

/**
 * A section label with its position in the pane's sequence. The account
 * pages are read top to bottom, each section unlocking the next, so the
 * labels are numbered over the sections actually shown.
 */
function StepLabel({
  n,
  children,
}: {
  n: number;
  children: ReactNode;
}): ReactNode {
  return (
    <SectionLabel className="step">
      <span className="n">{n}</span>
      {children}
    </SectionLabel>
  );
}

function MissingServerWarning({ action }: { action: ReactNode }): ReactNode {
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
  return username
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

/** Setup options for the initial screen: creating a new vault or joining an existing team. */
const JOINING_OPTIONS: readonly {
  path: FirstRunPath;
  icon: FoksIconName;
  title: string;
  detail: string;
}[] = [
  {
    path: 'own',
    icon: 'person',
    title: 'Set up my own account',
    detail: 'Set up your account and Personal vault.',
  },
  {
    path: 'invited',
    icon: 'people',
    title: 'Join an existing team',
    detail: 'Accept an invitation to join someone else’s team.',
  },
];

function JoiningChoice({
  value,
  onChange,
}: {
  value: FirstRunPath | null;
  onChange: (path: FirstRunPath) => void;
}): ReactNode {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  // Radio group keyboard navigation.
  const move = (from: number, step: number): void => {
    const next =
      (from + step + JOINING_OPTIONS.length) % JOINING_OPTIONS.length;
    onChange(JOINING_OPTIONS[next].path);
    refs.current[next]?.focus();
  };
  return (
    <div className="opts" role="radiogroup" aria-label="Setup method">
      {JOINING_OPTIONS.map((option, index) => {
        const on = value === option.path;
        return (
          <button
            key={option.path}
            ref={(node) => {
              refs.current[index] = node;
            }}
            type="button"
            role="radio"
            aria-checked={on}
            tabIndex={on || (value === null && index === 0) ? 0 : -1}
            className={on ? 'opt on' : 'opt'}
            onClick={() => onChange(option.path)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
                event.preventDefault();
                move(index, 1);
              } else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
                event.preventDefault();
                move(index, -1);
              }
            }}
          >
            <span className="pip" aria-hidden="true" />
            <Icon name={option.icon} className="ic" />
            <span className="txt">
              <span className="ptitle">{option.title}</span>
              <span className="bd">{option.detail}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
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
      facts?.deviceName ??
      suggestedDeviceName(),
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
  const [backupPhrase, setBackupPhrase] = useState<string | null>(null);
  const [phraseWritten, setPhraseWritten] = useState(false);
  // Pending path selection before confirmation.
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const pendingRef = useRef<PendingOperation[]>([]);
  const [otherMutationBusy, setBusy] = useState(false);
  const [phraseOperation, setPhraseOperation] = useState<symbol | null>(null);
  const phraseOwner = useRef<symbol | null>(null);
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
  // The failure the message on screen came from, so a banner can offer the
  // raw chain behind it. Only `fail` sets it, and the reason is offered only
  // while the text it reported is still the text being shown.
  const [lastFailure, setLastFailure] = useState<FirstRunFailure | null>(null);
  const backupPreparation = useRef<{
    key: string;
    promise: Promise<{ backupAlias: string; phrase: string }>;
  } | null>(null);
  const profile = checkpoint.profile;
  const fail = useCallback(
    (
      operation: FirstRunOperation,
      error: unknown,
      report: (message: string) => void = setMessage,
      isCurrent: () => boolean = () => mounted.current,
      onFailure?: (failure: FirstRunFailure) => void,
    ): void => {
      if (!isCurrent()) return;
      const failure = classifyFirstRunFailure(operation, error);
      setLastFailure(failure);
      onFailure?.(failure);
      // The command layer sets its write gate when a first-run mutation
      // returns an ambiguous or response-binding result, and only a fresh
      // catalog load releases it. Without this refresh the user is stuck on
      // "Refresh the vault..." until the app restarts, so reconcile here and
      // re-read pending operations so a committed-but-unacknowledged signup
      // can still be resumed.
      if (failure.recovery !== 'pending') {
        report(failure.error.message);
        return;
      }
      report(presentFirstRunFailure(failure).detail);
      // A first server check can set the gate before a profile has been
      // selected. Only the pending-operation read needs a profile.
      void reconcileFirstRunFailure(failure, async () => {
        if (!isCurrent()) return;
        await onRefreshSnapshot();
        if (!isCurrent()) return;
        if (profile) {
          const rows = await enqueueProfileWork<PendingOperation[] | null>(
            bridge,
            profile.profile,
            () =>
              isCurrent()
                ? bridge.listPendingOperations(profile.profile)
                : Promise.resolve(null),
          );
          if (!isCurrent() || !rows) return;
          pendingRef.current = rows;
          setPending(rows);
        }
      }).then((reconciled) => {
        if (!isCurrent()) return;
        onFailure?.(reconciled);
        report(presentFirstRunFailure(reconciled).detail);
      });
    },
    [bridge, mounted, onRefreshSnapshot, profile],
  );
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
    if (checkpoint.account?.deviceName || facts?.deviceName) return;
    const fallback = suggestedDeviceName();
    let alive = true;
    void bridge
      .appInfo()
      .then((info) => {
        const name = info.computerName?.trim();
        if (!alive || !name) return;
        setDeviceName((current) => (current === fallback ? name : current));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [bridge, checkpoint.account?.deviceName, facts?.deviceName]);

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
      setMessage(null);
      setDuplicateAlias(null);
      setConnectionErrors({ copy: null, recover: null, pair: null });
      setRecoveryPhrase('');
      setPairingPhrase('');
      setInvite('');
      if (next !== 'protect' && next !== 'phrase') {
        setPassphrase('');
        setConfirmation('');
        backupPreparation.current = null;
        setBackupPhrase(null);
        setPhraseWritten(false);
      } else if (state === 'phrase' && next !== 'phrase') {
        if (!checkpoint.backupCommitted) {
          backupPreparation.current = null;
          setBackupPhrase(null);
        }
        setPhraseWritten(false);
      }
      const current = checkpointRef.current;
      if (next === setupBackTarget(current)) send({ type: 'back' });
      else if (next === 'account' || next === 'existing')
        send({
          type: 'select-account-method',
          method: next === 'existing' ? 'recover' : 'create',
        });
      else send({ type: 'navigate', state: next });
    },
    [checkpoint.backupCommitted, checkpointRef, send, setDuplicateAlias, state],
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
      { deviceName, username, email, invite, phrase: recoveryPhrase },
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
      deviceName,
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
    backupPhrase,
  );

  const clearSecrets = useCallback((): void => {
    phraseOwner.current = null;
    setPhraseOperation(null);
    setInvite('');
    setPassphrase('');
    setConfirmation('');
    setRecoveryPhrase('');
    setPairingPhrase('');
    backupPreparation.current = null;
    setBackupPhrase(null);
    setPhraseWritten(false);
    secretsHeld.current = false;
  }, []);

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

  useEffect(() => {
    const concealWhenHidden = (): void => {
      if (document.visibilityState !== 'hidden') return;
      clearSecrets();
      if (state === 'phrase') go('protect');
    };
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () =>
      document.removeEventListener('visibilitychange', concealWhenHidden);
  }, [clearSecrets, go, state]);

  useEffect(() => {
    if (!agentReady || !profile) {
      if (pendingRef.current.length > 0) {
        pendingRef.current = [];
        setPending([]);
      }
      return;
    }
    let alive = true;
    void enqueueProfileWork(bridge, profile.profile, () =>
      bridge.listPendingOperations(profile.profile),
    ).then(
      (rows) => {
        if (!alive) return;
        const current = pendingRef.current;
        const unchanged =
          current.length === rows.length &&
          current.every(
            (entry, index) =>
              entry.kind === rows[index]?.kind &&
              entry.alias === rows[index]?.alias &&
              entry.target === rows[index]?.target,
          );
        if (!unchanged) {
          pendingRef.current = rows;
          setPending(rows);
        }
        if (checkpoint.returning) {
          const recoveries = rows.filter(
            (row) => row.kind === 'account-recovery' && !row.target,
          );
          if (recoveries.length === 1) {
            const resumableAlias = recoveries[0]?.alias;
            if (resumableAlias) {
              setUsername((current) => current || resumableAlias);
            }
          }
        }
      },
      (error) => {
        if (alive) fail('pending-read', error);
      },
    );
    return () => {
      alive = false;
    };
  }, [agentReady, bridge, checkpoint.returning, fail, profile]);

  useEffect(() => {
    if (
      !agentReady ||
      state !== 'phrase' ||
      backupPhrase ||
      !profile ||
      !accountAlias
    )
      return;
    const owner = Symbol('phrase preparation');
    phraseOwner.current = owner;
    setPhraseOperation(owner);
    const key = `${profile.profile}\u0000${accountAlias}`;
    const attempt =
      backupPreparation.current?.key === key
        ? backupPreparation.current.promise
        : bridge.prepareOwnerBackup(profile.profile, accountAlias, 'paper');
    backupPreparation.current = { key, promise: attempt };
    void attempt.then(
      (result) => {
        if (phraseOwner.current === owner) {
          setBackupPhrase(result.phrase);
          setPhraseOperation(null);
        }
      },
      (error) => {
        if (phraseOwner.current !== owner) return;
        backupPreparation.current = null;
        setPhraseOperation(null);
        go('protect');
        fail('backup-prepare', error);
      },
    );
    return () => {
      if (phraseOwner.current === owner) phraseOwner.current = null;
      setPhraseOperation((current) => (current === owner ? null : current));
    };
  }, [
    accountAlias,
    agentReady,
    backupPhrase,
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
    if (!profile || !username.trim() || !deviceName.trim() || !accountAlias)
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
              deviceName: deviceName.trim(),
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
      !deviceName.trim() ||
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
              deviceName.trim(),
            );
          else
            await bridge.recoverOwnerAccount(
              profile.profile,
              accountAlias,
              recoveryPhrase,
              deviceName.trim(),
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
      !deviceName.trim()
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
                deviceName.trim(),
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
      if (!next.backupCommitted && backupPhrase && phraseWritten) {
        await bridge.commitOwnerBackup(
          profile.profile,
          accountAlias,
          'paper',
          backupPhrase,
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
    if (!profile || !backupPhrase || !phraseWritten) return;
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
        backupPhrase,
      );
      setPhraseWritten(false);
      send({ type: 'backup-committed' });
    } catch (error) {
      fail('backup-commit', error);
    } finally {
      setBusy(false);
    }
  };

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
          section: 'servers',
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
        />
      </InsetRow>
    </>
  );
  const aliasInvalidNotice = usernameAliasInvalid ? (
    <p className="crit" role="alert">
      Your name must contain at least one letter or number.
    </p>
  ) : null;
  /* Sign-in authentication methods: backup phrase recovery, or credential
     import and pairing when an eligible FOKS CLI profile is detected. When only
     one method is available, it is selected automatically. */
  const signinMethods: SigninMethod[] = [
    'recover',
    ...(goCandidate?.copyable ? (['import'] as const) : []),
    ...(goCandidate?.pairable ? (['pair'] as const) : []),
  ];
  const pendingFor = (kind: PendingOperation['kind']): boolean =>
    pending.some((row) => row.kind === kind && row.alias === accountAlias);
  const signinMethod: SigninMethod | null =
    chosenSigninMethod && signinMethods.includes(chosenSigninMethod)
      ? chosenSigninMethod
      : signinMethods.length === 1
        ? 'recover'
        : signinMethods.includes('pair') && pendingFor('pairing-acceptance')
          ? 'pair'
          : pendingFor('account-recovery')
            ? 'recover'
            : null;
  /* The page foot's primary action follows the method. Recovery and pairing
     need their phrase typed first; import needs only the alias. */
  const signinPrimary =
    signinMethod === 'recover' ? (
      <Button
        variant="primary"
        disabled={
          busy || !accountAlias || !recoveryPhrase.trim() || !deviceName.trim()
        }
        onClick={() => void recover()}
      >
        {pendingFor('account-recovery') ? 'Resume recovery' : 'Recover'}
      </Button>
    ) : signinMethod === 'import' ? (
      <Button
        variant="primary"
        disabled={busy || !accountAlias}
        onClick={() => void copyGoCandidate()}
      >
        Import credentials
      </Button>
    ) : signinMethod === 'pair' ? (
      <Button
        variant="primary"
        disabled={
          busy || !accountAlias || !deviceName.trim() || !pairingPhrase.trim()
        }
        onClick={() => void acceptPairing(false)}
      >
        Accept pairing
      </Button>
    ) : (
      <Button variant="primary" disabled>
        Continue
      </Button>
    );
  const methodError = (key: keyof typeof connectionErrors): ReactNode => {
    const text = connectionErrors[key];
    return text ? (
      <p className="crit" role="alert">
        <FailureText text={text} reason={reasonFor(text)} />
      </p>
    ) : null;
  };
  /* The sign-in pages' numbered sections: the method, then, once one is
     chosen, the account and device rows with the method's own row and notes.
     `first` is the number of the method section, since Set up your account
     draws its setup-method group before these. */
  const signinSections = (first: number): ReactNode => (
    <>
      <StepLabel n={first}>Sign in method</StepLabel>
      <Inset>
        <RadioGroup label="Sign-in method">
          <RadioCard
            title="Recover with your backup phrase"
            detail="Enter all 17 words from your backup phrase to restore full access on this device."
            selected={signinMethod === 'recover'}
            onSelect={() => setChosenSigninMethod('recover')}
          />
          {goCandidate?.copyable ? (
            <RadioCard
              title="Import this device’s FOKS CLI credentials"
              detail="Both apps share the same device credentials. This may require a Keychain prompt."
              selected={signinMethod === 'import'}
              onSelect={() => setChosenSigninMethod('import')}
            />
          ) : null}
          {goCandidate?.pairable ? (
            <RadioCard
              title="Use the CLI to approve this as a new device"
              detail={
                <>
                  Pair with <code>foks --simple-ui key assist</code> in
                  Terminal.
                </>
              }
              selected={signinMethod === 'pair'}
              onSelect={() => setChosenSigninMethod('pair')}
            />
          ) : null}
        </RadioGroup>
      </Inset>
      {/* If the account already exists, display its details as read-only fields
          regardless of the selected sign-in method. */}
      {signinMethod || checkpoint.account ? (
        <>
          <StepLabel n={first + 1}>Account and device</StepLabel>
          {signinMethod === 'pair' ? (
            <>
              <p className="hint">
                In Terminal, switch the official FOKS CLI to this account, then
                run:
              </p>
              <CopyBox
                text="foks --simple-ui key assist"
                onCopy={(value) =>
                  void bridge
                    .copyText(value)
                    .then(() => toasts.show('Command copied.'))
                }
              >
                <code>foks --simple-ui key assist</code>
              </CopyBox>
            </>
          ) : null}
          <Inset className="account-form">
            {identityRows}
            {signinMethod === 'recover' ? (
              <InsetRow label="Phrase">
                <input
                  type="password"
                  aria-label="Backup phrase"
                  value={recoveryPhrase}
                  onChange={(event) => setRecoveryPhrase(event.target.value)}
                />
              </InsetRow>
            ) : null}
            {signinMethod === 'pair' ? (
              <InsetRow label="Pairing phrase">
                <input
                  type="password"
                  aria-label="Pairing phrase"
                  placeholder="Enter pairing phrase"
                  value={pairingPhrase}
                  onChange={(event) => setPairingPhrase(event.target.value)}
                />
              </InsetRow>
            ) : null}
          </Inset>
          {aliasInvalidNotice}
          {signinMethod === 'import' ? (
            <p className="hint">
              Both apps will share the same device credentials. This may require
              a Keychain prompt. Revoking the device in either client will
              disable both.
            </p>
          ) : null}
          {signinMethod === 'pair' ? (
            <p className="hint">
              Select the account in the CLI, enter the pairing phrase above, and
              follow the terminal prompts to complete pairing. Or,{' '}
              <button
                type="button"
                className="lnk"
                aria-label="Resume pairing"
                disabled={busy || !accountAlias || !deviceName.trim()}
                onClick={() => void acceptPairing(true)}
              >
                resume pairing
              </button>{' '}
              for an existing account.
            </p>
          ) : null}
          {signinMethod
            ? methodError(
                signinMethod === 'recover'
                  ? 'recover'
                  : signinMethod === 'import'
                    ? 'copy'
                    : 'pair',
              )
            : null}
        </>
      ) : null}
    </>
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
          <MissingServerWarning
            action={
              abortable ? (
                <Button variant="danger" onClick={discardProvisioning}>
                  Start over
                </Button>
              ) : null
            }
          />
        ) : null}
        {operationResumable && intent?.kind === 'recovery' ? (
          <div className="local-field-card">
            <label className="local-field-row">
              <span>Backup phrase</span>
              <input
                type="password"
                value={recoveryPhrase}
                onChange={(event) => setRecoveryPhrase(event.target.value)}
              />
            </label>
          </div>
        ) : null}
        <div className="actions operation-actions">
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
        {abortable && operationProblem !== 'profile-missing' ? (
          <div className="alt-path">
            <div className="t">
              <b>Start over</b>
              <span>
                Starting over won’t delete any account already created on the
                server. You can find existing accounts on the Account tab.
              </span>
            </div>
            <Button variant="danger" onClick={discardProvisioning}>
              Start over
            </Button>
          </div>
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
        <div className="actions">
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
              <code>{managedStatus?.configuredProbe ?? 'localhost:4430'}</code>
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
        <div className="actions">
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
        <div className="actions">
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
        <div className="actions">
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
      <Pane
        title={checkpoint.path === 'invited' ? 'Team Server' : 'Server Setup'}
        header={false}
        scope={
          state === 'no-address' && checkpoint.path === 'invited'
            ? 'Finding your server address'
            : undefined
        }
        foot={
          <Foot back={() => go('who')}>
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void checkServer()}
            >
              Continue
            </Button>
          </Foot>
        }
      >
        <h1>
          {checkpoint.path === 'invited'
            ? 'Select a server address'
            : 'Select a server'}
        </h1>
        <p className="lead">
          FOKS synchronizes your account, teams, and encrypted vaults through a
          server.{' '}
          {checkpoint.path === 'invited'
            ? `Enter the server address provided by ${adminShort}.`
            : null}
        </p>
        <Inset
          className={state === 'error' || addressInvalid ? 'err' : undefined}
        >
          <label className="fr server-address-row">
            <span className="k">Address</span>
            <span className="v">
              <input
                ref={serverAddressInput}
                aria-label="Server address"
                value={address}
                placeholder="e.g. foks.app:4430"
                onChange={(event) => {
                  editServerAddress(event.target.value);
                }}
              />
            </span>
          </label>
        </Inset>
        <Button
          className="official-server"
          disabled={busy}
          onClick={() => {
            editServerAddress('foks.app:4430');
          }}
        >
          Use the official FOKS server
        </Button>
        {state === 'error' && !addressInvalid ? (
          <div className="crit" role="alert">
            <b>
              {serverCheckPresentation?.title ??
                message ??
                (address
                  ? `Could not connect to ${address}`
                  : 'No server address provided')}
            </b>
            <p>
              {serverCheckPresentation?.detail ??
                'Confirm the address is correct, then retry.'}
            </p>
            {serverCheckPresentation?.reason ? (
              <Toggle label="Details">
                <pre>{serverCheckPresentation.reason}</pre>
              </Toggle>
            ) : null}
          </div>
        ) : null}
        {checkpoint.path === 'invited' && state === 'no-address' ? (
          <div className="pcard">
            <h3>{`Ask ${adminShort} for the server address`}</h3>
            <p>
              Contact them through your usual communication channel. This is the
              only detail needed right now.
            </p>
            <CopyBox
              text="What’s the address of the FOKS server our team is on?"
              onCopy={(value) =>
                void bridge
                  .copyText(value)
                  .then(() => toasts.show('Copied to clipboard.'))
              }
            >
              “What’s the address of the FOKS server our team is on?”
            </CopyBox>
            <p>
              In the next step, you will choose a username and share it with
              them so they can add you to the team.
            </p>
          </div>
        ) : checkpoint.path === 'invited' ? (
          <p className="hint">
            <button className="lnk" onClick={() => go('no-address')}>
              Don’t have a server address?
            </button>
          </p>
        ) : null}
      </Pane>
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
                  (!username.trim() || !deviceName.trim() || !accountAlias))
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
    content = (
      <Pane
        title="Your account"
        header={false}
        scope={
          signingIn
            ? 'Device authorization required'
            : 'Keys are generated securely on your device.'
        }
        wide
        foot={
          <Foot
            back={() =>
              go(
                signingIn
                  ? checkpoint.account
                    ? 'protect'
                    : accountBackTarget
                  : accountBackTarget,
              )
            }
          >
            {signingIn ? (
              checkpoint.account ? (
                <Button onClick={() => go('protect')}>
                  Skip to latest step
                </Button>
              ) : (
                signinPrimary
              )
            ) : ssoSelected ? (
              <span className="foot-slot" ref={setSsoPrimarySlot} />
            ) : (
              <Button
                variant="primary"
                disabled={
                  busy ||
                  usernameAliasInvalid ||
                  (!checkpoint.account &&
                    (!username.trim() || !deviceName.trim()))
                }
                // If the account was already created when returning from Protect, proceed to protection.
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
            )}
          </Foot>
        }
      >
        <h1>Set up your account</h1>
        <p className="lead">
          {signingIn ? (
            <>
              Your account already exists on {profile?.canonicalName}. Recover
              it with your backup phrase
              {goCandidate ? ' or connect using the official FOKS CLI' : ''}.
            </>
          ) : (
            'Your account keys are generated on this device; only the public keys are sent to the server.'
          )}
        </p>
        <StepLabel n={1}>Setup method</StepLabel>
        <Inset>
          <RadioGroup label="Account setup">
            <div className="choice">
              <RadioCard
                title="Create a new account"
                detail="Set up a new FOKS account on this device."
                selected={!signingIn && !ssoSelected}
                disabled={Boolean(checkpoint.sso) || cliAccountSelected}
                off={cliAccountSelected}
                onSelect={() => {
                  go('account');
                }}
              />
              {!signingIn && !ssoSelected ? (
                <div className="choice-body">
                  <Toggle
                    label="Email or invite code"
                    defaultOpen={Boolean(email || invite)}
                  >
                    <Inset className="account-form">
                      <InsetRow label="Email">
                        <input
                          value={email}
                          placeholder="you@example.net"
                          onChange={(event) => setEmail(event.target.value)}
                        />
                      </InsetRow>
                      <InsetRow label="Invite code">
                        <input
                          value={invite}
                          onChange={(event) => setInvite(event.target.value)}
                        />
                      </InsetRow>
                    </Inset>
                  </Toggle>
                </div>
              ) : null}
            </div>
            {ssoAvailable && profile ? (
              <div className="choice">
                <RadioCard
                  title="Sign up with your organization"
                  detail="Create the account through your organization’s identity provider."
                  selected={ssoSelected}
                  disabled={cliAccountSelected}
                  off={cliAccountSelected}
                  onSelect={() => {
                    clearSecrets();
                    send({
                      type: 'select-account-method',
                      method: 'organization',
                    });
                  }}
                />
                {ssoSelected ? (
                  <div className="choice-body">
                    <Toggle label="Invite code" defaultOpen={Boolean(invite)}>
                      <Inset className="account-form">
                        <InsetRow label="Invite code">
                          <input
                            value={invite}
                            onChange={(event) => setInvite(event.target.value)}
                          />
                        </InsetRow>
                      </Inset>
                    </Toggle>
                    <SsoPanel
                      key={`${profile.profile}/${accountAlias}`}
                      embedded
                      primarySlot={ssoPrimarySlot}
                      bridge={bridge}
                      profile={profile.profile}
                      account={accountAlias}
                      login={false}
                      deviceName={deviceName}
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
                          [
                            'cancelled',
                            'expired',
                            'denied',
                            'rejected',
                          ].includes(progress.state) &&
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
                          deviceName,
                        );
                      }}
                    />
                  </div>
                ) : null}
              </div>
            ) : null}
            <div className="choice">
              <RadioCard
                title="Sign in to an existing account"
                detail="Add this device to an account you already have."
                selected={signingIn}
                onSelect={() => {
                  if (signingIn) return;
                  openExisting();
                }}
              />
            </div>
          </RadioGroup>
        </Inset>
        {signingIn ? (
          signinSections(2)
        ) : (
          <>
            <StepLabel n={2}>Account and device</StepLabel>
            <Inset className="account-form">{identityRows}</Inset>
            {aliasInvalidNotice}
            {message ? (
              <p className="crit" role="alert">
                <FailureText text={message} reason={reasonFor(message)} />
              </p>
            ) : null}
            {duplicateAlias && !busy && !identityLoading ? (
              <div className="band info" role="status">
                <span className="t">
                  “{duplicateAlias.alias}” is already set up on this device for
                  this server. Use that account, or choose a different username.
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
          </>
        )}
      </Pane>
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
          with your backup phrase
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
        backupPhrase={backupPhrase}
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
  else if (state === 'waiting' && checkpoint.selectedGroup)
    content = (
      <Pane
        title="Team vault unavailable"
        foot={
          <Foot>
            <Button onClick={() => go('checklist-invited')}>
              Finish later
            </Button>
          </Foot>
        }
      >
        <h1>{checkpoint.selectedGroup.name}</h1>
        <p className="lead">
          {/* Reached either from a vault that could not be opened or from a
              team the catalog no longer binds, which the checkpoint does not
              tell apart; the retry below finds out which. */}
          {message ||
            'This team’s vault could not be opened. Retry to check your membership again, or choose another team.'}
        </p>
        <div className="actions">
          <Button
            variant="primary"
            disabled={busy}
            busy={busy}
            onClick={() => void discover()}
          >
            {busy ? 'Retrying…' : 'Retry loading team'}
          </Button>
          <Button
            onClick={() => {
              commit({
                ...checkpoint,
                selectedGroup: undefined,
                group: undefined,
                added: false,
              });
              setMessage(null);
            }}
          >
            Choose another team
          </Button>
        </div>
      </Pane>
    );
  else if (state === 'waiting')
    content = (
      <Pane
        title={`Waiting for ${admin} to add ${checkpoint.account?.username}`}
        header={false}
        wide
        foot={
          <Foot>
            <Button onClick={() => go('checklist-invited')}>
              Finish later
            </Button>
          </Foot>
        }
      >
        <h1>
          Waiting for {admin} to add {checkpoint.account?.username}
        </h1>
        <p className="lead">
          Your account ({checkpoint.account?.username}) on{' '}
          {profile?.canonicalName} is ready. {adminShort} must add you to{' '}
          <b>{group}</b>. Once they have added you, select Check now to finish
          joining.
        </p>
        {discoveredGroups.length > 1 ? (
          <div className="pcard">
            <h3>Choose an existing team</h3>
            <p>
              You’re already a member of these teams. Open one to get started.
            </p>
            <div className="btns">
              {discoveredGroups.map((found) => (
                <Button
                  key={`${found.kind}:${found.teamIdHex}:${found.alias}`}
                  onClick={() => selectDiscoveredGroup(found)}
                >
                  Open “{found.name ?? found.alias}”
                </Button>
              ))}
            </div>
          </div>
        ) : null}
        <div className="two">
          <div className="col">
            <div className="pcard">
              <h3>Message for {adminShort}</h3>
              <CopyBox
                text={`Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`}
                onCopy={(value) =>
                  void bridge
                    .copyText(value)
                    .then(() => toasts.show('Copied to clipboard.'))
                }
              >
                “Add {checkpoint.account?.username} on {profile?.canonicalName}{' '}
                to {group}”
              </CopyBox>
              <p>
                Send this message to {adminShort} so they have your exact
                username and server.
              </p>
            </div>
            <Inset className="checklist">
              <InsetRow label={<Icon name="people" />}>
                <b>{group} will appear under Teams</b>
                <span className="hint">
                  Teams appear once membership is confirmed by the server.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="eye" />}>
                <b>Access depends on your team role</b>
                <span className="hint">
                  Roles include <b>Member</b>, <b>Admin</b>, and <b>Owner</b>.
                  You can only access items permitted by your assigned role and
                  visibility level.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="door" />}>
                <b>You can close FOKS anytime</b>
                <span className="hint">
                  Your account and server settings are saved on this device.
                  When you reopen FOKS, you can continue setup.
                </span>
              </InsetRow>
            </Inset>
          </div>
          <div className="col">
            <div className="pcard">
              <h3>Check now</h3>
              <div className="checkrow">
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() => void discover()}
                >
                  Check now
                </Button>
                <span className="status">
                  {/* Only a check that completed and found nothing says the
                      team was not found; a failed check says it failed, and
                      the sentence below carries what went wrong. */}
                  <Chip>
                    {!message
                      ? 'Not checked yet'
                      : discoveryOutcome === 'failed'
                        ? 'Check failed'
                        : 'Checked: now'}
                  </Chip>
                  {message && discoveryOutcome === 'not-found' ? (
                    <>Team not found yet. Only Personal is available.</>
                  ) : null}
                </span>
              </div>
              <p>
                Select <b>Check now</b> to look for pending team invitations.
                FOKS will also check automatically each time you open the app.
              </p>
              <Band label="Team updates">
                <b>Check now</b> checks the server for team memberships linked
                to your account. FOKS also checks when it opens.
              </Band>
              {message ? <div className="res">{message}</div> : null}
              <p className="note">
                You can use <b>Personal</b> while you wait. If {adminShort} has
                already added you, confirm that they entered{' '}
                <code>{checkpoint.account?.username}</code>.
              </p>
              <details className="dd">
                <summary>
                  <Icon name="chev" />
                  Details
                </summary>
                <p>
                  Checks your account ({checkpoint.account?.username}) on{' '}
                  {profile?.canonicalName} and updates your team list.
                </p>
              </details>
            </div>
            <div className="pcard">
              <h3>Use your Personal vault</h3>
              <p>
                {PERSONAL_FIXED} You can store private items in <b>Personal</b>{' '}
                right away; items in Personal are never shared with {group}.
              </p>
              <Button onClick={() => go('checklist-invited')}>
                Continue to my vault
              </Button>
            </div>
          </div>
        </div>
      </Pane>
    );
  else if (state === 'checklist-invited' || state === 'checklist-own') {
    const stepsDone = completedFirstRunSteps(checkpoint);
    const stepsTotal = firstRunStepCount(checkpoint);
    const recoverySet = Boolean(
      checkpoint.passphraseSet || checkpoint.backupCommitted,
    );
    // Completed checklist items display one summary line; server connection
    // details are available in Settings. If recovery setup was skipped, the
    // warning banner explains that unbacked accounts cannot be recovered.
    content = (
      <Pane
        title="Get started"
        subtitle={`${stepsDone} of ${stepsTotal} steps completed`}
      >
        <p className="lead">
          {stepsDone === stepsTotal
            ? 'Your account is ready. Start using your Personal vault.'
            : 'Completed steps are saved. Finish account recovery below, or start using your Personal vault.'}
        </p>
        {personalRefreshError ? (
          <p className="crit" role="alert">
            {personalRefreshError}
          </p>
        ) : null}
        <Inset className="checklist">
          <InsetRow label="✓">
            <b>
              {checkpoint.path === 'invited'
                ? 'Their server'
                : 'Select a server'}
            </b>
            <span className="hint">Using {profile?.canonicalName}</span>
          </InsetRow>
          <InsetRow label="✓">
            <b>Your account</b>
            <span className="hint">
              {checkpoint.accountMethod === 'recover'
                ? 'Restored account as'
                : 'Created as'}{' '}
              {checkpoint.account?.username}
            </span>
          </InsetRow>
          <InsetRow
            className={recoverySet ? undefined : 'skipped'}
            label={recoverySet ? '✓' : '!'}
          >
            <b>Save recovery phrase</b>
            <span className="hint">
              {recoverySet
                ? [
                    checkpoint.backupCommitted ? 'Backup phrase saved' : null,
                    checkpoint.passphraseSet ? 'Passphrase set' : null,
                  ]
                    .filter(Boolean)
                    .join(' · ')
                : 'Backup method not configured'}
            </span>
          </InsetRow>
          {checkpoint.path === 'invited' ? (
            <InsetRow
              className={checkpoint.added ? undefined : 'pending'}
              label={checkpoint.added ? '✓' : '4'}
              action={
                <span className="checklist-actions">
                  <Button
                    size="sm"
                    icon="copy"
                    onClick={() =>
                      void bridge
                        .copyText(
                          `Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`,
                        )
                        .then(() => toasts.show('Message copied.'))
                    }
                  >
                    Copy message
                  </Button>
                  <Button size="sm" onClick={() => go('waiting')}>
                    Check now
                  </Button>
                </span>
              }
            >
              <b>Waiting for {admin} to add you</b>
              <span className="hint">
                Check again after {adminShort} adds you to {group}.
              </span>
              <Band label="Team discovery">
                Check now looks up teams for this signed-in account.
              </Band>
            </InsetRow>
          ) : null}
        </Inset>
        {recoverySet ? null : (
          <div className="checklist-notice">
            <Band
              label={`Recovery keys for ${checkpoint.account?.username} are only saved on this device.`}
              action={
                <Button
                  size="sm"
                  variant="primary"
                  onClick={() => go('protect')}
                >
                  Set up recovery
                </Button>
              }
            >
              Without a backup method, this account can’t be recovered if this
              device is lost.
            </Band>
          </div>
        )}
        <div className="checklist-cta">
          {!accountStore ? (
            <Button disabled={personalRefreshing} onClick={retryPersonal}>
              {personalRefreshing
                ? 'Loading Personal vault…'
                : 'Retry loading Personal vault'}
            </Button>
          ) : null}
          <Button
            variant="primary"
            disabled={!accountStore}
            onClick={() => {
              if (accountStore)
                onNavigate({ kind: 'store', ref: accountStore });
            }}
          >
            Continue to my vault
          </Button>
        </div>
      </Pane>
    );
  } else
    content = (
      <Pane title={group} subtitle={`Team on ${profile?.canonicalName}`} wide>
        <Notice
          title={addedStore ? `Joined ${group}` : `${group}: vault unavailable`}
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
                {item.path.split('/').at(-1)}
                <small>{item.path}</small>
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
        <div className="actions">
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
            title: 'Start setup over?',
            body: 'The setup in progress on this device is discarded. Accounts already created on the server are kept.',
            confirm: 'Start over',
          }}
          onConfirm={startSetupOver}
          onCancel={() => setConfirmingRestart(false)}
        />
      ) : null}
    </>
  );
}
