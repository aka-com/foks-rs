import { LocalCompleteStep } from './first-run-complete-step';
import { ServerVerificationStep } from './first-run-server-step';
import { RecoveryStep } from './first-run-recovery-step';
import {
  stepOf,
  SetupSidebar,
  FirstRunAppSidebar,
  AddedDetails,
  Foot,
  Pane,
} from './first-run-view';
export { FirstRunChecklistStatus } from './first-run-view';
import { useFirstRunController } from '../use-first-run-controller';
import {
  clearRetainedSetups,
  retainSetup,
  retainedSetups,
  sameSetupTarget,
  updateRetainedSetup,
} from '../first-run-recovery';
import { resolveSetupEntry, setupActions } from '../first-run-controller';
import { SsoPanel } from '../components/sso-panel';
import {
  resolveProvisionedIdentity,
  provisionedIdentityProblem,
  identityProblemText,
  type IdentityProblem,
} from '../first-run-identity';
import {
  executeProvisioning,
  provisioningInFlight,
  persistFirstRun,
} from '../first-run-operations';
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
  CheckedProfileResponse,
  GoProfileCandidate,
  GoProfileDiscovery,
  DiscoveredGroup,
  PendingOperation,
  ServerStatusSnapshot,
} from '../bridge';
import {
  enqueueProfileWork,
  isAgentReadinessError,
  normalizeCommandError,
  sharedServerStatus,
} from '../bridge';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  completedFirstRunSteps,
  firstRunStepCount,
  decodeFirstRunCheckpoint,
  initialFirstRun,
  isFirstRunState,
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

/**
 * The profile identifier a server address suggests: the host as letters,
 * digits, and dashes, keeping a port only when it is not the default. So
 * "foks.app:4430" is `foks-app` and "localhost:5000" is `localhost-5000`.
 */
export function profileNameFor(address: string): string {
  const host = address
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, '')
    .replace(/\/.*$/, '');
  // An IPv6 literal keeps its groups but would otherwise collapse to a run of
  // dashes, so it is named for what it is: "[fe80::1]:4430" is `ipv6-fe80-1`.
  const ipv6 = /^\[([^\]]+)\](?::\d+)?$/.exec(host);
  const bare = ipv6 ? `ipv6-${ipv6[1]}` : host.replace(/:4430$/, '');
  return (
    bare
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 52) || 'server'
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

/** Delay between automatic retries of a transient identity refresh failure. */
const IDENTITY_RETRY_DELAY_MS = 2_000;
/** Bounded so a mutation that never finishes still surfaces its error. */
const IDENTITY_RETRY_LIMIT = 30;

/**
 * Text to show while an identity refresh failure that clears on its own is
 * retried, or null for failures that need the user.
 */
function transientIdentityFailure(code: string): string | null {
  switch (code) {
    case 'mutation-in-flight':
      return 'Waiting for the current operation to finish…';
    case 'catalog-required':
      return 'The vault changed while loading. Trying again…';
    default:
      return null;
  }
}

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

function fixtureSeed(
  bridge: Bridge,
  path: FirstRunPath,
  state: FirstRunStateName,
): FirstRunCheckpoint {
  const facts = bridge.firstRunFixture?.[path];
  let next = initialFirstRun(path, state);
  if (state === 'boot') return next;
  if (state === 'error' && facts) next = { ...next, serverAddress: facts.typo };
  if (
    (stepOf(state) >= 2 || state === 'checked' || state === 'compare') &&
    facts
  ) {
    next = { ...next, profile: facts.report, serverAddress: facts.server };
  }
  if (stepOf(state) >= 3 && facts) {
    next = {
      ...next,
      account: {
        alias: facts.accountAlias,
        username: facts.username,
        deviceName: facts.deviceName,
      },
    };
  }
  if (state === 'identity-pending' && facts)
    next = {
      ...next,
      provisionedAccount: {
        alias: facts.accountAlias,
        deviceName: facts.deviceName,
      },
    };
  if (stepOf(state) >= 4 && state !== 'phrase')
    next = { ...next, passphraseSet: true, backupCommitted: true };
  if (state === 'added')
    next = {
      ...next,
      added: true,
      group:
        facts?.groupName && facts.groupAlias && facts.groupTeamIdHex
          ? {
              name: facts.groupName,
              kind: 'named',
              alias: facts.groupAlias,
              teamIdHex: facts.groupTeamIdHex,
            }
          : undefined,
    };
  if (state === 'checklist-invited')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      added: false,
    };
  if (state === 'checklist-own')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      group: undefined,
    };
  return next;
}

function initialCheckpoint(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  location: Extract<Location, { kind: 'first-run' }>,
  automaticEntry: boolean,
): FirstRunCheckpoint {
  const path: FirstRunPath = location.path ?? 'invited';
  let saved: FirstRunCheckpoint | null = null;
  try {
    saved = decodeFirstRunCheckpoint(
      window.localStorage.getItem(FIRST_RUN_CHECKPOINT_KEY),
    );
  } catch {
    /* unavailable storage */
  }
  const queryState =
    typeof window === 'undefined'
      ? null
      : new URLSearchParams(window.location.search).get('state');
  const namedReviewState = isFirstRunState(queryState);
  if (bridge.firstRunFixture && namedReviewState)
    return fixtureSeed(bridge, path, location.step as FirstRunStateName);
  const state =
    location.step === 'boot' || !isFirstRunState(location.step)
      ? 'who'
      : location.step;
  return resolveSetupEntry(
    snapshot,
    saved,
    { state, path: location.path },
    automaticEntry,
  );
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
  onReplaceSession,
  sessionEntry,
}: FirstRunExperienceProps & {
  onReplaceSession: (next: FirstRunCheckpoint) => void;
  sessionEntry: FirstRunCheckpoint | null;
}): ReactNode {
  const toasts = useToast();
  const { checkpoint, checkpointRef, mounted, commit, send } =
    useFirstRunController({
      initial: () =>
        sessionEntry ??
        initialCheckpoint(bridge, snapshot, location, automaticEntry),
      snapshot,
      bridge,
      location,
      onNavigate,
      agentReady,
    });
  const setupEnvironment = useRef({ snapshot, onRefreshSnapshot });
  setupEnvironment.current = { snapshot, onRefreshSnapshot };
  const facts = bridge.firstRunFixture?.[checkpoint.path];
  const [address, setAddress] = useState(
    () => checkpoint.serverAddress ?? facts?.server ?? '',
  );
  const [addressInvalid, setAddressInvalid] = useState(false);
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
  const [goDiscovery, setGoDiscovery] = useState<GoProfileDiscovery | null>(
    null,
  );
  const [goCandidate, setGoCandidate] = useState<GoProfileCandidate | null>(
    null,
  );
  const goCandidateExplicit = useRef(false);
  const [goChooserDismissed, setGoChooserDismissed] = useState(false);
  const [goScanError, setGoScanError] = useState<string | null>(null);
  const [goScanAttempt, setGoScanAttempt] = useState(0);
  const [backupPhrase, setBackupPhrase] = useState<string | null>(null);
  const [phraseWritten, setPhraseWritten] = useState(false);
  // Pending path selection before confirmation.
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const [discoveredGroups, setDiscoveredGroups] = useState<DiscoveredGroup[]>(
    [],
  );
  const discoveredSnapshot = useRef<AgentSnapshot>(snapshot);
  const pendingRef = useRef<PendingOperation[]>([]);
  const [mutationBusy, setMutationBusy] = useState(false);
  const [busyOperation, setBusyOperation] = useState<
    'server-check' | 'other' | null
  >(null);
  const setBusy = useCallback(
    (value: boolean, operation: 'server-check' | 'other' = 'other'): void => {
      setMutationBusy(value);
      setBusyOperation(value ? operation : null);
    },
    [],
  );
  const [phraseOperation, setPhraseOperation] = useState<symbol | null>(null);
  const phraseOwner = useRef<symbol | null>(null);
  const busy = mutationBusy || phraseOperation !== null;
  const [identityLoading, setIdentityLoading] = useState(false);
  const [identityError, setIdentityError] = useState<string | null>(null);
  // Shown while a transient identity refresh failure is retried on its own.
  const [identityWaiting, setIdentityWaiting] = useState<string | null>(null);
  const identityRetry = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (identityRetry.current !== null)
        window.clearTimeout(identityRetry.current);
    },
    [],
  );
  const [identityProblem, setIdentityProblem] =
    useState<IdentityProblem | null>(null);
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
  const autoProbedKey = useRef<string | null>(null);
  const identityGeneration = useRef(0);
  const [personalRefreshing, setPersonalRefreshing] = useState(false);
  const [personalRefreshError, setPersonalRefreshError] = useState<
    string | null
  >(null);
  const [agentRetrying, setAgentRetrying] = useState(false);
  const [agentRetryError, setAgentRetryError] = useState<string | null>(null);
  const [connectionErrors, setConnectionErrors] = useState<
    Record<'copy' | 'recover' | 'pair', string | null>
  >({ copy: null, recover: null, pair: null });
  const addressRevision = useRef(0);
  const serverAddressInput = useRef<HTMLInputElement>(null);
  const addressSelection = useRef<{
    start: number | null;
    end: number | null;
  } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  // The failure the message on screen came from, so a banner can offer the
  // raw chain behind it. Only `fail` sets it, and the reason is offered only
  // while the text it reported is still the text being shown.
  const [lastFailure, setLastFailure] = useState<FirstRunFailure | null>(null);
  const [serverCheckFailure, setServerCheckFailure] =
    useState<FirstRunFailure | null>(null);
  const [managedStatus, setManagedStatus] =
    useState<ServerStatusSnapshot | null>(null);
  const [managedStatusError, setManagedStatusError] = useState<string | null>(
    null,
  );
  const [managedStatusAttempt, setManagedStatusAttempt] = useState(0);
  const backupPreparation = useRef<{
    key: string;
    promise: Promise<{ backupAlias: string; phrase: string }>;
  } | null>(null);

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
  const profile = checkpoint.profile;
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

  useEffect(() => {
    if (!agentReady || state !== 'local') return;
    setManagedStatus(null);
    if (!managedProfile) {
      setManagedStatusError(
        'No local server is running on this device. Connect to an existing server to continue.',
      );
      return;
    }
    let alive = true;
    setManagedStatusError(null);
    const server = setupEnvironment.current.snapshot.servers.find(
      (row) => row.id === managedProfile,
    );
    if (server?.trust.status === 'blocked' || server?.restrictions.length) {
      setManagedStatusError(
        server.trust.status === 'blocked'
          ? server.trust.error.message
          : server.restrictions[0].error.message,
      );
      return;
    }
    void sharedServerStatus(bridge, managedProfile, true).then(
      async (status) => {
        if (!alive) return;
        if (
          status.profile !== managedProfile ||
          status.configuredProbe !== 'localhost:4430' ||
          status.leaseRequired ||
          !status.host
        ) {
          setManagedStatusError('The local server is not ready.');
          return;
        }
        // Cached connectivity failure must not suppress a live local probe.
        // Publish readiness only after the catalog agrees with its pinned host.
        try {
          const refreshed =
            await setupEnvironment.current.onRefreshSnapshot(true);
          if (!alive) return;
          const current = refreshed.servers.filter(
            (row) => row.id === managedProfile,
          );
          if (
            current.length !== 1 ||
            current[0].host_id !== status.host.hostId ||
            current[0].trust.status === 'blocked' ||
            current[0].restrictions.length
          ) {
            setManagedStatusError(
              'The local server responded, but its saved identity or configuration needs attention. Review server settings.',
            );
            return;
          }
          setManagedStatus(status);
        } catch (error) {
          if (alive)
            setManagedStatusError(
              `The local server responded, but its account list could not be refreshed: ${normalizeCommandError(error).message}`,
            );
        }
      },
      (error) => {
        if (alive) setManagedStatusError(normalizeCommandError(error).message);
      },
    );
    return () => {
      alive = false;
    };
  }, [bridge, managedProfile, managedStatusAttempt, state, agentReady]);

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
    [checkpoint.backupCommitted, checkpointRef, send, state],
  );
  const openExisting = useCallback((): void => {
    go('existing');
  }, [go]);
  const fail = useCallback(
    (
      operation: FirstRunOperation,
      error: unknown,
      report: (message: string) => void = setMessage,
    ): void => {
      const failure = classifyFirstRunFailure(operation, error);
      setLastFailure(failure);
      if (operation === 'server-check') setServerCheckFailure(failure);
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
        await onRefreshSnapshot();
        if (profile) {
          const rows = await enqueueProfileWork(bridge, profile.profile, () =>
            bridge.listPendingOperations(profile.profile),
          );
          pendingRef.current = rows;
          setPending(rows);
        }
      }).then((reconciled) => {
        if (operation === 'server-check')
          setServerCheckFailure((current) =>
            current === failure ? reconciled : current,
          );
        report(presentFirstRunFailure(reconciled).detail);
      });
    },
    [bridge, onRefreshSnapshot, profile],
  );

  useEffect(() => {
    if (!agentReady) setIdentityLoading(false);
    const generation = identityGeneration;
    return () => {
      generation.current++;
    };
  }, [agentReady]);

  const refreshAccountIdentity = async (
    saved: FirstRunCheckpoint = checkpointRef.current,
    attempt = 0,
  ): Promise<void> => {
    if (!saved.provisionedAccount || !agentReady) return;
    autoProbedKey.current = `identity:${saved.provisionedAccount.alias}`;
    const generation = ++identityGeneration.current;
    if (identityRetry.current !== null) {
      window.clearTimeout(identityRetry.current);
      identityRetry.current = null;
    }
    let retrying = false;
    setIdentityLoading(true);
    setIdentityError(null);
    setIdentityWaiting(null);
    setIdentityProblem(null);
    try {
      // Account mutations invalidate the native catalog. Do not join an
      // in-flight read that may have started before the account was copied or
      // created, or identity adoption can remain stuck on that stale result.
      const refreshed = await sharedSetupRead(
        bridge,
        `identity:${saved.profile?.profile}:${saved.provisionedAccount.alias}`,
        () => onRefreshSnapshot(true),
      );
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const resolved = resolveProvisionedIdentity(refreshed, saved);
      if (resolved !== saved) commit(resolved);
      else {
        const problem =
          provisionedIdentityProblem(refreshed, saved) ??
          'inventory-unavailable';
        setIdentityProblem(problem);
        setIdentityError(identityProblemText[problem]);
      }
    } catch (error) {
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const typed = normalizeCommandError(error);
      if (isAgentReadinessError(typed)) {
        setIdentityError(typed.message);
        onAgentReadinessFailure?.(typed);
        return;
      }
      // A native mutation still holding the catalog, or a snapshot replaced
      // by a concurrent load, clears on its own. Wait and try again rather
      // than presenting a vault-screen message as a setup failure.
      const waiting = transientIdentityFailure(typed.code);
      if (waiting && attempt < IDENTITY_RETRY_LIMIT) {
        retrying = true;
        setIdentityWaiting(waiting);
        identityRetry.current = window.setTimeout(() => {
          identityRetry.current = null;
          if (!mounted.current || generation !== identityGeneration.current)
            return;
          if (checkpointRef.current !== saved) {
            setIdentityWaiting(null);
            setIdentityLoading(false);
            return;
          }
          void refreshAccountIdentity(saved, attempt + 1);
        }, IDENTITY_RETRY_DELAY_MS);
        return;
      }
      setIdentityError(typed.message);
    } finally {
      if (generation === identityGeneration.current && !retrying)
        setIdentityLoading(false);
    }
  };

  const accountProvisioned = (alias: string): void => {
    const saved = transitionFirstRun(checkpointRef.current, {
      type: 'account-provisioned',
      alias,
      deviceName: deviceName.trim(),
    });
    // Persist the acknowledgement before any fallible inventory read.
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
    kind: ProvisioningIntent['kind'],
    alias: string,
    operation: () => Promise<unknown>,
    extra: Partial<
      Pick<ProvisioningIntent, 'candidateId' | 'ssoOperationId'>
    > = {},
    details: { resume?: boolean; phrase?: string } = {},
  ): Promise<void> => {
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
                  ? { phrase: details.phrase ?? recoveryPhrase }
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
    let receiptError: string | null = null;
    const withReceipt = (text: string): string =>
      receiptError ? `${text} (Details: ${receiptError})` : text;
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
          receiptError = typed.message;
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
        setOperationStatus(withReceipt(identityProblemText[problem]));
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
        withReceipt(
          resumable
            ? 'Account setup was interrupted. Resume it to continue.'
            : adoptable
              ? 'An account with this username already exists on this server. You can use this account, start over, or check your server settings.'
              : 'Account setup still could not be confirmed. Check again, start over, or check your server settings.',
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
  }, [agentReady, busy, identityLoading, mounted, probeKey]);

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
    setOperationStatus(null);
    setOperationProblem(null);
    setOperationResumable(false);
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
    setIdentityError(null);
    setIdentityProblem(null);
  };

  const resumeAccountOperation = async (): Promise<void> => {
    const saved = checkpointRef.current;
    const intent = saved.provisioning;
    if (!intent || !saved.profile || !operationResumable || busy) return;
    const name = saved.profile.profile;
    await runAccountOperation(intent.kind, intent.alias, () => {
      if (intent.kind === 'signup')
        return bridge.resumeFirstRunAccount(name, intent.alias);
      if (intent.kind === 'recovery')
        return bridge.resumeOwnerRecovery(
          name,
          intent.alias,
          recoveryPhrase,
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
    setRecoveryPhrase('');
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
   * of them — that is what Finish later leaves behind.
   */
  const leaveDisabled =
    mutationBusy &&
    busyOperation !== 'server-check' &&
    !checkpoint.provisioning;

  const recoveryActions = setupActions(checkpoint, {
    mutating: mutationBusy,
    operationRunning,
    inFlight: Boolean(
      checkpoint.provisioning &&
      provisioningInFlight(bridge, checkpoint.provisioning.id),
    ),
  });
  const [restartError, setRestartError] = useState<string | null>(null);
  const replaceSession = (next: FirstRunCheckpoint): void => {
    persistFirstRun(next);
    clearSecrets();
    mounted.current = false;
    identityGeneration.current++;
    onReplaceSession(next);
  };
  /**
   * Setup is re-entered from the app sidebar with an account already set up,
   * so the attempt in hand is retained rather than discarded.
   */
  const reenterSetup = (): void => {
    if (!recoveryActions.canRestart) return;
    try {
      retainSetup(checkpointRef.current);
      replaceSession(initialFirstRun(checkpoint.path));
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
    }
  };
  const [confirmingRestart, setConfirmingRestart] = useState(false);
  /** Discards this device’s setup, retained attempts included. */
  const startSetupOver = (): void => {
    setConfirmingRestart(false);
    if (!recoveryActions.canRestart) return;
    try {
      clearRetainedSetups();
      replaceSession(initialFirstRun(checkpoint.path));
    } catch (error) {
      setRestartError(normalizeCommandError(error).message);
    }
  };

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

  useLayoutEffect(() => {
    const selection = addressSelection.current;
    if (!selection) return;
    addressSelection.current = null;
    serverAddressInput.current?.focus();
    serverAddressInput.current?.setSelectionRange(
      selection.start,
      selection.end,
    );
  }, [address, state]);

  const editUsername = (value: string): void => {
    setUsername(value);
    setDuplicateAlias(null);
  };

  const editServerAddress = (value: string): void => {
    const input = serverAddressInput.current;
    if (input && document.activeElement === input)
      addressSelection.current = {
        start: input.selectionStart,
        end: input.selectionEnd,
      };
    if (!goCandidateExplicit.current) setGoCandidate(null);
    addressRevision.current++;
    setAddress(value);
    setAddressInvalid(false);
    setMessage(null);
    setServerCheckFailure(null);
    send({ type: 'server-edited', address: value });
  };

  const checkServer = async (): Promise<void> => {
    if (!agentReady) return;
    if (!address.trim()) {
      setAddressInvalid(true);
      if (state === 'error') send({ type: 'navigate', state: 'address' });
      return;
    }
    const revision = addressRevision.current;
    setAddressInvalid(false);
    setBusy(true, 'server-check');
    setMessage(null);
    setServerCheckFailure(null);
    try {
      const profileName = facts?.profile ?? profileNameFor(address);
      const report = goCandidate
        ? await bridge.checkAndAddGoProfile(
            goCandidate.candidateId,
            goCandidate.hostId,
            profileName,
            address.trim(),
          )
        : await bridge.checkAndAddProfile(profileName, address.trim());
      if (revision !== addressRevision.current) return;
      if (goCandidate && report.hostId !== goCandidate.hostId)
        throw new Error(
          'The server response does not match the selected profile.',
        );
      // A profile made here is labelled with the server's own name, so the rail
      // reads "foks.app" rather than the identifier derived from the address.
      if (!facts?.profile && report.canonicalName) {
        try {
          await bridge.setServerLabel(profileName, report.canonicalName);
        } catch {
          // The label is cosmetic; failing to set it does not fail the check.
        }
      }
      send({
        type: 'profile-checked',
        address: address.trim(),
        profile: report,
      });
    } catch (error) {
      if (revision !== addressRevision.current) return;
      fail('server-check', error);
      send({ type: 'navigate', state: 'error' });
    } finally {
      setBusy(false);
    }
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

  const selectDiscoveredGroup = (
    found: DiscoveredGroup,
    refreshed: AgentSnapshot,
  ): void => {
    if (
      !bridge.firstRunFixture &&
      refreshed.servers.filter(
        (server) =>
          server.id === profile?.profile && server.host_id === profile?.hostId,
      ).length !== 1
    ) {
      setMessage(
        'The server identity could not be confirmed. Review server settings before opening this team.',
      );
      return;
    }
    const identity = {
      name: found.name ?? found.alias,
      kind: found.kind,
      alias: found.alias,
      teamIdHex: found.teamIdHex,
    };
    const selected = {
      ...checkpointRef.current,
      state: 'waiting' as const,
      selectedGroup: identity,
    };
    const stores = refreshed.stores.filter(
      (store) =>
        store.kind === 'team' &&
        store.active &&
        store.server === profile?.profile &&
        store.account === checkpoint.account?.alias &&
        store.team_id_hex === found.teamIdHex &&
        store.alias === found.alias &&
        store.team_kind === found.kind,
    );
    if (stores.length !== 1 || !storeReadable(refreshed, stores[0].id)) {
      commit(selected);
      setMessage(
        'Team located, but the vault is currently unavailable. Retry loading the vault, or complete setup later.',
      );
      return;
    }
    commit(
      transitionFirstRun(selected, {
        type: 'group-discovered',
        group: { ...identity, name: stores[0].name },
      }),
    );
  };

  const discover = async (): Promise<void> => {
    if (!agentReady || !profile || !checkpoint.account || busy) return;
    const saved = checkpointRef.current;
    setBusy(true);
    setMessage(null);
    setDiscoveredGroups([]);
    try {
      let result: Awaited<ReturnType<Bridge['discoverGroups']>>;
      let refreshed: AgentSnapshot;
      try {
        result = await enqueueProfileWork(bridge, profile.profile, () =>
          bridge.discoverGroups(profile.profile, checkpoint.account!.alias),
        );
      } finally {
        // Discovery invalidates the catalog even for zero groups or errors.
        refreshed = await onRefreshSnapshot(true);
      }
      if (
        !mounted.current ||
        checkpointRef.current.profile?.hostId !== saved.profile?.hostId ||
        checkpointRef.current.account?.alias !== saved.account?.alias
      )
        return;
      if (
        result.accountAlias !== checkpoint.account.alias ||
        result.groups.some(
          (row) => row.accountAlias !== checkpoint.account?.alias,
        )
      )
        throw new Error(
          'Team discovery returned data for a different account.',
        );
      const eligible = result.groups.filter(
        (candidate) =>
          candidate.active &&
          (!facts?.groupName || candidate.name === facts.groupName),
      );
      discoveredSnapshot.current = refreshed;
      const unique = new Set(
        eligible.map((row) => `${row.kind}:${row.teamIdHex}:${row.alias}`),
      );
      if (unique.size !== eligible.length)
        throw new Error('Team discovery returned conflicting team records.');
      const selected = checkpointRef.current.selectedGroup;
      const match = selected
        ? eligible.filter(
            (row) =>
              row.teamIdHex === selected.teamIdHex &&
              row.alias === selected.alias &&
              row.kind === selected.kind,
          )
        : eligible;
      if (match.length === 1) selectDiscoveredGroup(match[0], refreshed);
      else if (match.length > 1) {
        setDiscoveredGroups(match);
        setMessage(
          'Choose the team you want to open. Your other memberships will remain available.',
        );
      } else
        setMessage(
          selected
            ? 'Membership in the selected team could not be confirmed. Check again or choose another team.'
            : 'No active teams found yet. You can use Personal while you wait.',
        );
    } catch (error) {
      if (mounted.current) fail('group-discovery', error);
    } finally {
      if (mounted.current) setBusy(false);
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

  const managedReport: CheckedProfileResponse | null = useMemo(() => {
    const host = managedStatus?.host;
    if (!managedProfile || !host) return null;
    return {
      profile: managedProfile,
      acceptance: 'unchanged',
      lookupName: host.lookupName,
      canonicalName: host.canonicalName,
      hostId: host.hostId,
      chain: host.chain,
      epoch: host.epoch,
    };
  }, [managedProfile, managedStatus]);

  const selectManagedProfile = (returning = false): void => {
    if (!agentReady || !managedReport || !managedStatus) return;
    const current = snapshot.servers.filter(
      (server) => server.id === managedReport.profile,
    );
    if (
      current.length !== 1 ||
      current[0].host_id !== managedReport.hostId ||
      current[0].trust.status === 'blocked' ||
      current[0].restrictions.length
    ) {
      setManagedStatusError(
        'Server settings changed. Check the local server again.',
      );
      return;
    }
    send({
      type: 'managed-profile-selected',
      address: managedStatus.configuredProbe,
      profile: managedReport,
      returning,
    });
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
  /* The ways into an account that already exists: recovery by backup phrase,
     and, with an eligible FOKS CLI profile, credential import or pairing.
     Drawn under the "Sign in to an existing account" choice on Select an
     account, and on the managed-local path's own page. */
  const recoverButton = (
    <Button
      variant="primary"
      disabled={
        busy || !accountAlias || !recoveryPhrase.trim() || !deviceName.trim()
      }
      onClick={() => void recover()}
    >
      {pending.some(
        (row) => row.kind === 'account-recovery' && row.alias === accountAlias,
      )
        ? 'Resume recovery'
        : 'Recover'}
    </Button>
  );
  /* The account alias and this device’s name identify the same account
     whichever way it is reached, so both account pages draw them once, above
     the ways in. */
  const identityFields = (
    <>
      <SectionLabel>Account and device</SectionLabel>
      <Inset className="account-form">
        <InsetRow label="Account alias">
          <input
            aria-label="Account alias"
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
      </Inset>
    </>
  );
  const aliasInvalidNotice = usernameAliasInvalid ? (
    <p className="crit">
      Account alias must contain at least one letter or number.
    </p>
  ) : null;
  // The ways into an existing account. Recovery's primary button is
  // `recoverButton`, drawn in the page foot; the CLI cards keep their own.
  const existingCards = (
    <div className="signin-methods">
      <div className="pcard">
        <h3>Recover with your backup phrase</h3>
        <p>
          Enter all 17 words from your backup phrase to restore full access on
          this device.
        </p>
        <Inset className="recovery-fields">
          <InsetRow label="Phrase">
            <input
              type="password"
              aria-label="Backup phrase"
              value={recoveryPhrase}
              onChange={(event) => setRecoveryPhrase(event.target.value)}
            />
          </InsetRow>
        </Inset>
        {connectionErrors.recover ? (
          <p className="crit" role="alert">
            <FailureText
              text={connectionErrors.recover}
              reason={reasonFor(connectionErrors.recover)}
            />
          </p>
        ) : null}
      </div>
      {goCandidate?.copyable ? (
        <div className="pcard">
          <h3>Import this device’s FOKS CLI credentials</h3>
          <p>
            Both apps will share the same device credentials. This may require a
            Keychain prompt. Revoking the device in either client will disable
            both.
          </p>
          <Button
            variant="primary"
            className="copy-device"
            disabled={busy || !accountAlias}
            onClick={() => void copyGoCandidate()}
          >
            Import credentials
          </Button>
          {connectionErrors.copy ? (
            <p className="crit" role="alert">
              <FailureText
                text={connectionErrors.copy}
                reason={reasonFor(connectionErrors.copy)}
              />
            </p>
          ) : null}
        </div>
      ) : null}
      {goCandidate?.pairable ? (
        <div className="pcard">
          <h3>Use the CLI to approve this as a new device</h3>
          <p>
            In Terminal, switch the official FOKS CLI to this account, then run:
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
          <p>
            Select the account in the CLI, enter the pairing code below, and
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
            a past account.
          </p>
          <Inset className="recovery-fields">
            <InsetRow label="Pairing phrase">
              <input
                type="password"
                aria-label="Pairing phrase"
                placeholder="Enter pairing phrase"
                value={pairingPhrase}
                onChange={(event) => setPairingPhrase(event.target.value)}
              />
            </InsetRow>
          </Inset>
          <div className="btns">
            <Button
              variant="primary"
              disabled={
                busy ||
                !accountAlias ||
                !deviceName.trim() ||
                !pairingPhrase.trim()
              }
              onClick={() => void acceptPairing(false)}
            >
              Accept pairing
            </Button>
          </div>
          {connectionErrors.pair ? (
            <p className="crit" role="alert">
              <FailureText
                text={connectionErrors.pair}
                reason={reasonFor(connectionErrors.pair)}
              />
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
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
            ? 'Account setup is running. You can finish later while it completes.'
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
        <div className="actions">
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
          <div className="crit">
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
  else if (state === 'who' && !goChooserDismissed && goCandidates.length > 0)
    content = (
      <Pane title="Select an FOKS account" header={false} wide>
        <h1>Select an FOKS account</h1>
        <p className="lead">
          Choose an account from the official FOKS CLI. In the next step, you
          can connect this app as a new device or copy your existing device
          credentials.
        </p>
        <GoProfileChooser
          candidates={goCandidates}
          selected={goCandidate?.candidateId ?? null}
          onSelect={(candidate) => {
            goCandidateExplicit.current = true;
            setGoCandidate(candidate);
          }}
        />
        <div className="actions">
          <Button
            variant="primary"
            disabled={!goCandidate}
            onClick={() => {
              if (!goCandidate) return;
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
          <Button
            onClick={() => {
              goCandidateExplicit.current = false;
              setGoCandidate(null);
              setGoChooserDismissed(true);
            }}
          >
            Create a new account
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
          <div className="crit">
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
              Use this server
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
          FOKS stores your account, teams, and encrypted vaults on a server.{' '}
          {checkpoint.path === 'invited'
            ? `Enter the server address provided by ${adminShort}.`
            : null}
        </p>
        <SectionLabel>Server</SectionLabel>
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
          <div className="crit">
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
          <p className="crit">
            Username must contain at least one letter or number.
          </p>
        ) : null}
        {message ? (
          <p className="crit">
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
    // "Sign in to an existing account" selected and its cards under the radios.
    const signingIn = state === 'existing';
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
                <Button onClick={() => go('protect')}>Resume protection</Button>
              ) : (
                recoverButton
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
        {identityFields}
        {aliasInvalidNotice}
        <SectionLabel>Setup method</SectionLabel>
        <Inset>
          <RadioGroup label="Account setup">
            <div className="choice">
              <RadioCard
                title="Create a new account"
                detail="Set up a new FOKS account on this device."
                selected={!signingIn && !ssoSelected}
                disabled={Boolean(checkpoint.sso)}
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
                        accountProvisioned(accountAlias);
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
              {signingIn ? (
                <div className="choice-body">{existingCards}</div>
              ) : null}
            </div>
          </RadioGroup>
        </Inset>
        {signingIn ? null : (
          <>
            {message ? (
              <p className="crit">
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
  // its Create your account pane has no radios to draw the cards under.
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
              <Button onClick={() => go('protect')}>Resume protection</Button>
            ) : (
              <>
                <Button
                  onClick={() => {
                    go('account');
                  }}
                >
                  Create a new account
                </Button>
                {recoverButton}
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
        {identityFields}
        {aliasInvalidNotice}
        {existingCards}
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
          {message ||
            'Your team was found, but its vault is not available yet.'}
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
                  onClick={() =>
                    selectDiscoveredGroup(found, discoveredSnapshot.current)
                  }
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
                  <Chip>{message ? 'Checked: now' : 'Not checked yet'}</Chip>
                  {message ? (
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
                Open Personal
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
            <b>{checkpoint.path === 'invited' ? 'Their server' : 'A server'}</b>
            <span className="hint">
              <code>{profile?.canonicalName}</code>
            </span>
          </InsetRow>
          <InsetRow label="✓">
            <b>Your account</b>
            <span className="hint">
              <code>{checkpoint.account?.username}</code> ·{' '}
              {checkpoint.account?.deviceName}
            </span>
          </InsetRow>
          <InsetRow
            className={recoverySet ? undefined : 'skipped'}
            label={recoverySet ? '✓' : '!'}
            action={
              recoverySet ? (
                <Button size="sm" onClick={() => go('protect')}>
                  Review
                </Button>
              ) : undefined
            }
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
                : 'Skipped — backup method not configured'}
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
            Open Personal
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
        />
      ) : (
        <SetupSidebar
          checkpoint={checkpoint}
          native={Boolean(bridge.native)}
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
