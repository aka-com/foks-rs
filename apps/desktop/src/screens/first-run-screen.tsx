import { SsoPanel } from '../components/sso-panel';
import { resolveProvisionedIdentity } from '../first-run-identity';
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
  SectionLabel,
  SheetDialog,
  Toggle,
} from '../components';
import type { FilterKind } from '../components';
import type {
  Bridge,
  CommandError,
  CheckedProfileResponse,
  GoProfileCandidate,
  GoProfileDiscovery,
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
  encodeFirstRunCheckpoint,
  initialFirstRun,
  isFirstRunState,
  reconcileFirstRunCheckpoint,
  transitionFirstRun,
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
import { NavRow, Sidebar } from '../shell/sidebar';
import {
  formatRole,
  kindOf,
  parseRole,
  plural,
  profileInventoryComplete,
  serverAvailability,
  storeReadable,
} from '../model';
import type { RoleWire, World } from '../model';
import { PageHeader } from '../shell/page-header';
import { GoProfileChooser } from './go-profile-chooser';
import { readableBy } from './scope';
import { useToast } from '/kit/toasts';

const PERSONAL_FIXED =
  'Your Personal vault is private to your account. To share items with others, use a group.';

const stepOf = (state: FirstRunStateName): number => {
  if (state === 'boot') return 0;
  if (state === 'local') return 0;
  if (state === 'who') return 0;
  if (['address', 'no-address', 'checked', 'compare', 'error'].includes(state))
    return 1;
  if (
    state === 'account' ||
    state === 'existing' ||
    state === 'identity-pending'
  )
    return 2;
  if (state === 'protect' || state === 'phrase') return 3;
  if (state === 'waiting') return 4;
  return 5;
};

const localStepOf = (state: FirstRunStateName): number => {
  if (state === 'local') return 0;
  if (
    state === 'account' ||
    state === 'existing' ||
    state === 'identity-pending'
  )
    return 1;
  if (state === 'protect' || state === 'phrase') return 2;
  return 3;
};

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

function authoritativeSetupFacts(
  world: World,
  checkpoint: FirstRunCheckpoint,
): Parameters<typeof reconcileFirstRunCheckpoint>[1] {
  const server = checkpoint.profile
    ? world.servers.find(
        (candidate) => candidate.id === checkpoint.profile?.profile,
      )
    : undefined;
  const profile = checkpoint.profile
    ? !profileInventoryComplete(world, 'profiles')
      ? 'unknown'
      : !server
        ? 'missing'
        : server.host_id === null
          ? 'unknown'
          : server.host_id === checkpoint.profile.hostId
            ? 'present'
            : 'missing'
    : 'unknown';
  const account = checkpoint.account
    ? profileInventoryComplete(world, 'accounts')
      ? world.accounts.some(
          (candidate) =>
            candidate.server === checkpoint.profile?.profile &&
            candidate.alias === checkpoint.account?.alias,
        )
        ? 'present'
        : 'missing'
      : 'unknown'
    : 'unknown';
  const group = checkpoint.group
    ? profileInventoryComplete(world, 'teams')
      ? world.stores.some(
          (candidate) =>
            candidate.kind === 'team' &&
            candidate.server === checkpoint.profile?.profile &&
            candidate.alias === checkpoint.group?.alias &&
            candidate.team_id_hex === checkpoint.group.teamIdHex,
        )
        ? 'present'
        : 'missing'
      : 'unknown'
    : 'unknown';
  return { profile, account, group };
}

function initialCheckpoint(
  bridge: Bridge,
  world: World,
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
  const reconcile = (checkpoint: FirstRunCheckpoint): FirstRunCheckpoint =>
    resolveProvisionedIdentity(
      world,
      reconcileFirstRunCheckpoint(
        checkpoint,
        authoritativeSetupFacts(world, checkpoint),
      ),
    );
  // URL/navigation state must never offer an acknowledged mutation again.
  if (saved?.provisionedAccount) return reconcile(saved);
  if (location.step === 'identity-pending')
    return saved ? reconcile(saved) : initialFirstRun(path, 'who');
  if (automaticEntry && saved) return reconcile(saved);
  // Phrase state is not persisted across reloads; resume at 'protect'.
  const state =
    location.step === 'phrase'
      ? 'protect'
      : location.step === 'boot'
        ? 'who'
        : (location.step as FirstRunStateName);
  if (
    !saved ||
    (location.path !== undefined && location.path !== saved.path) ||
    (state === 'who' && saved.state !== 'who')
  ) {
    return initialFirstRun(path, state);
  }
  return reconcile({ ...saved, path: location.path ?? saved.path, state });
}

function SetupSidebar({
  checkpoint,
  pendingPath,
  onCancel,
  onAnotherServer,
  onRecoverAccount,
  recoverEnabled = false,
}: {
  checkpoint: FirstRunCheckpoint;
  /** Path selected on the 'who' screen before confirmation. */
  pendingPath?: FirstRunPath | null;
  onCancel?: () => void;
  onAnotherServer?: () => void;
  onRecoverAccount?: () => void;
  recoverEnabled?: boolean;
}): ReactNode {
  if (checkpoint.managedLocal) {
    const current = localStepOf(checkpoint.state);
    const labels = ['Local server', 'Create account', 'Account recovery'];
    return (
      <nav className="side setup-side" aria-label="Setup steps">
        <div className="setup-steps">
          {labels.map((label, index) => (
            <div
              key={label}
              className={`setup-step${index === current ? ' on' : ''}${index < current ? ' done' : ''}`}
              aria-current={index === current ? 'step' : undefined}
            >
              <span className="setup-mark" aria-hidden="true">
                {index < current ? '✓' : index + 1}
              </span>
              <span className="t">{label}</span>
            </div>
          ))}
        </div>
        {onAnotherServer || onRecoverAccount || onCancel ? (
          <div className="foot">
            {onAnotherServer ? (
              <button type="button" className="nav" onClick={onAnotherServer}>
                <Icon name="server" />
                <span className="t">Connect to another server</span>
              </button>
            ) : null}
            {onRecoverAccount ? (
              <button
                type="button"
                className="nav"
                disabled={!recoverEnabled}
                onClick={onRecoverAccount}
              >
                <Icon name="person" />
                <span className="t">Recover account</span>
              </button>
            ) : null}
            {onCancel ? (
              <button type="button" className="nav" onClick={onCancel}>
                <Icon name="x" />
                <span className="t">Leave setup</span>
              </button>
            ) : null}
          </div>
        ) : null}
      </nav>
    );
  }
  const current = stepOf(checkpoint.state);
  // Display neutral step labels until the user selects a path.
  const choosing = checkpoint.state === 'who';
  const path = choosing ? (pendingPath ?? null) : checkpoint.path;
  const labels = [
    'Get started',
    'Select a server',
    'Create account',
    'Protect account',
    ...(path === 'invited' ? ['Join a group'] : []),
    'Complete',
  ];
  return (
    <nav className="side setup-side" aria-label="Setup steps">
      <div className="setup-steps">
        {labels.map((label, index) => (
          <div
            key={label}
            className={`setup-step${index === current ? ' on' : ''}${index < current ? ' done' : ''}`}
            aria-current={index === current ? 'step' : undefined}
          >
            <span className="setup-mark">
              {index < current ? '✓' : index + 1}
            </span>
            <span className="t">{label}</span>
          </div>
        ))}
      </div>
      {onCancel ? (
        <div className="foot">
          <button type="button" className="nav" onClick={onCancel}>
            <Icon name="x" />
            <span className="t">Leave setup</span>
          </button>
        </div>
      ) : null}
    </nav>
  );
}

export function FirstRunChecklistStatus({
  checkpoint,
  active = false,
  onNavigate,
}: {
  checkpoint: FirstRunCheckpoint;
  active?: boolean;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const completed = completedFirstRunSteps(checkpoint);
  const total = firstRunStepCount(checkpoint);
  return (
    <>
      <SectionLabel as="side">Status</SectionLabel>
      {completed < total ? (
        <NavRow
          active={active}
          glyph={<Icon name="flag" />}
          name="Setup checklist"
          tail={
            <span className="badge">
              {completed} of {total}
            </span>
          }
          onSelect={() =>
            onNavigate({
              kind: 'first-run',
              step:
                checkpoint.path === 'invited'
                  ? 'checklist-invited'
                  : 'checklist-own',
              path: checkpoint.path,
            })
          }
        />
      ) : null}
    </>
  );
}

function FirstRunAppSidebar({
  world,
  checkpoint,
  groupName,
  location,
  onNavigate,
  onReenter,
}: {
  world: World;
  checkpoint: FirstRunCheckpoint;
  groupName: string;
  location: Location;
  onNavigate: (location: Location) => void;
  onReenter: () => void;
}): ReactNode {
  // Render setup progress in the sidebar status slot.
  const status = (
    <>
      <FirstRunChecklistStatus
        checkpoint={checkpoint}
        active
        onNavigate={onNavigate}
      />
      {checkpoint.path === 'invited' && !checkpoint.added ? (
        <p className="side-note">
          {groupName} will appear under Groups once your access is approved.
          FOKS checks for group access at launch. You can also click Check now.
        </p>
      ) : null}
    </>
  );
  return (
    <Sidebar
      world={world}
      location={location}
      alerts={checkpoint.path === 'invited' && !checkpoint.added ? 1 : 0}
      onNavigate={onNavigate}
      status={status}
      onReenter={onReenter}
    />
  );
}

function AddedDetails({
  world,
  storeId,
}: {
  world: World;
  storeId?: string;
}): ReactNode {
  const store = world.stores.find((candidate) => candidate.id === storeId);
  const item = world.items.find(
    (candidate) =>
      candidate.store === storeId && candidate.path.includes('staging-token'),
  );
  if (!item)
    return (
      <aside className="details">
        <div className="dh">
          <span className="t">
            <h2>Details</h2>
            <small>Select an item to view details</small>
          </span>
        </div>
      </aside>
    );
  const name = item.path.split('/').at(-1);
  const kind = kindOf(item) as FilterKind;
  return (
    <aside className="details">
      <div className="dh">
        <KindIcon kind={kind} />
        <span className="t">
          <h2>{name}</h2>
          <small>
            {kind.charAt(0).toUpperCase() + kind.slice(1)} in{' '}
            {store?.name ?? 'this group'}
          </small>
        </span>
      </div>
      <div className="scroll">
        <SectionLabel>Value</SectionLabel>
        <Inset variant="preview">
          <InsetRow
            label="Value"
            valueClass="mask"
            action={
              <Button size="sm" disabled>
                Show
              </Button>
            }
          >
            Locked
          </InsetRow>
        </Inset>
        <p className="pfn">
          Access to this item depends on your role in the group.
        </p>
        <SectionLabel>Info</SectionLabel>
        <div className="meta">
          <b>Location</b>
          <code>{item.path}</code>
          <b>Kind</b>
          <span>{kind}</span>
          <b>Version</b>
          <span>{item.version}</span>
          <b>Size</b>
          <span>
            {item.size === null
              ? 'Size unavailable'
              : plural(item.size, 'byte')}
          </span>
          <b>Read permission</b>
          <Chip>{roleText(item.read)}</Chip>
          <b>Write permission</b>
          <Chip>{roleText(item.write)}</Chip>
        </div>
        <SectionLabel>Sharing</SectionLabel>
        <p>
          Members of {store?.name ?? 'this group'} with the required role or
          higher can view this item based on the current member list.
        </p>
      </div>
    </aside>
  );
}

function roleText(role: RoleWire): string {
  const parsed = parseRole(role);
  if (parsed) return formatRole(parsed);
  return typeof role === 'string' ? role : role.role;
}

function Foot({
  children,
  back,
  note,
}: {
  children?: ReactNode;
  back?: () => void;
  note?: ReactNode;
}): ReactNode {
  return (
    <div className="pfoot">
      {back ? (
        <Button className="lnk" onClick={back}>
          Back
        </Button>
      ) : null}
      {note ? <span className="note">{note}</span> : null}
      <span className="spacer" />
      {children}
    </div>
  );
}

function Pane({
  title,
  subtitle,
  scope,
  header = true,
  wide = false,
  children,
  foot,
}: {
  title: string;
  /** Only rendered when `header` is true. */
  subtitle?: string;
  scope?: string;
  header?: boolean;
  wide?: boolean;
  children: ReactNode;
  foot?: ReactNode;
}): ReactNode {
  return (
    <>
      {header ? (
        <PageHeader
          title={title}
          subtitle={subtitle ?? ''}
          tail={scope ? <span className="scope">{scope}</span> : null}
        />
      ) : null}
      <div className="body pb">
        <div className={`pane${wide ? ' wide' : ''}`}>{children}</div>
      </div>
      {foot}
    </>
  );
}

/** Setup options for the initial screen: creating a new vault or joining an existing group. */
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
    title: 'Join an existing group',
    detail: 'Accept an invitation to join someone else’s group.',
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
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'first-run' }>;
  onNavigate: (location: Location) => void;
  onRefreshWorld: () => Promise<World>;
  concealSignal: number;
  agentReady: boolean;
  onRetryAgent?: () => Promise<void>;
  onAgentReadinessFailure?: (error: CommandError) => void;
  automaticEntry?: boolean;
  managedProfile?: string;
}

export function FirstRunExperience({
  world,
  bridge,
  location,
  onNavigate,
  onRefreshWorld,
  concealSignal,
  agentReady,
  onRetryAgent,
  onAgentReadinessFailure,
  automaticEntry = false,
  managedProfile,
}: FirstRunExperienceProps): ReactNode {
  const toasts = useToast();
  const [checkpoint, setCheckpoint] = useState(() =>
    initialCheckpoint(bridge, world, location, automaticEntry),
  );
  const checkpointRef = useRef(checkpoint);
  const [existingBack, setExistingBack] = useState<FirstRunStateName>(() =>
    checkpoint.returning
      ? checkpoint.managedLocal
        ? 'local'
        : 'checked'
      : 'account',
  );
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
  const [goChooserDismissed, setGoChooserDismissed] = useState(false);
  const [goScanError, setGoScanError] = useState<string | null>(null);
  const [recoveryAlias, setRecoveryAlias] = useState(
    () => checkpoint.account?.alias ?? facts?.accountAlias ?? '',
  );
  const [backupPhrase, setBackupPhrase] = useState<string | null>(null);
  const [phraseWritten, setPhraseWritten] = useState(false);
  // Pending path selection before confirmation.
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const pendingRef = useRef<PendingOperation[]>([]);
  const [mutationBusy, setBusy] = useState(false);
  const [phraseOperation, setPhraseOperation] = useState<symbol | null>(null);
  const phraseOwner = useRef<symbol | null>(null);
  const busy = mutationBusy || phraseOperation !== null;
  const [identityLoading, setIdentityLoading] = useState(false);
  const [identityError, setIdentityError] = useState<string | null>(null);
  const identityGeneration = useRef(0);
  const [personalRefreshing, setPersonalRefreshing] = useState(false);
  const [personalRefreshError, setPersonalRefreshError] = useState<
    string | null
  >(null);
  const [agentRetrying, setAgentRetrying] = useState(false);
  const [agentRetryError, setAgentRetryError] = useState<string | null>(null);
  const [accountBack, setAccountBack] = useState<FirstRunStateName | null>(
    null,
  );
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
  const [serverCheckFailure, setServerCheckFailure] =
    useState<FirstRunFailure | null>(null);
  const [managedStatus, setManagedStatus] =
    useState<ServerStatusSnapshot | null>(null);
  const [managedStatusError, setManagedStatusError] = useState<string | null>(
    null,
  );
  const [managedStatusAttempt, setManagedStatusAttempt] = useState(0);
  const automaticEntryPending = useRef(automaticEntry);
  const backupPreparation = useRef<{
    key: string;
    promise: Promise<{ backupAlias: string; phrase: string }>;
  } | null>(null);

  useEffect(() => {
    if (!automaticEntryPending.current) return;
    automaticEntryPending.current = false;
    onNavigate({
      kind: 'first-run',
      step: checkpoint.state,
      path: checkpoint.path,
    });
  }, [checkpoint.path, checkpoint.state, onNavigate]);

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

  useEffect(() => {
    if (
      !agentReady ||
      checkpoint.state !== 'who' ||
      !bridge.native ||
      goChooserDismissed
    )
      return;
    let alive = true;
    setGoScanError(null);
    void bridge
      .discoverGoProfiles()
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
    goChooserDismissed,
    onAgentReadinessFailure,
  ]);

  const state = checkpoint.state;
  const serverCheckPresentation = serverCheckFailure
    ? presentFirstRunFailure(serverCheckFailure)
    : null;
  const profile = checkpoint.profile;
  const accountAlias =
    checkpoint.account?.alias ??
    facts?.accountAlias ??
    username.toLowerCase().replace(/[^a-z0-9_-]+/g, '-');
  const recoveryTargetAlias = recoveryAlias.trim() || accountAlias;
  const goCandidates =
    goDiscovery?.candidates.filter(
      (candidate) => candidate.pairable || candidate.copyable,
    ) ?? [];
  const admin = facts?.admin ?? 'group administrator';
  const adminShort = facts?.admin
    ? facts.admin.split('.')[0]
    : 'the group administrator';
  const group = checkpoint.group?.name ?? facts?.groupName ?? 'your group';
  const addedStores = checkpoint.group
    ? world.stores.filter(
        (store) =>
          store.kind === 'team' &&
          store.active &&
          storeReadable(world, store.id) &&
          store.server === profile?.profile &&
          store.name === checkpoint.group?.name &&
          store.alias === checkpoint.group.alias &&
          store.team_kind === checkpoint.group.kind &&
          store.team_id_hex === checkpoint.group.teamIdHex,
      )
    : [];
  const addedStore = addedStores.length === 1 ? addedStores[0]?.id : undefined;
  const accountStores =
    checkpoint.account && profile
      ? world.accounts.filter(
          (candidate) =>
            candidate.server === profile.profile &&
            candidate.alias === checkpoint.account?.alias,
        )
      : [];
  const accountStore =
    accountStores.length === 1 ? accountStores[0]?.store : undefined;
  const personalAvailable = Boolean(
    accountStore && storeReadable(world, accountStore),
  );
  const accountItemCount = accountStore
    ? world.items.filter((item) => item.store === accountStore).length
    : 0;

  useEffect(() => {
    if (!agentReady || state !== 'local' || !managedProfile) return;
    setManagedStatus(null);
    let alive = true;
    setManagedStatusError(null);
    if (
      !world.servers.some(
        (server) =>
          server.id === managedProfile &&
          serverAvailability(world, server).available,
      )
    ) {
      setManagedStatusError(
        'The local server is not responding. Make sure the background service is running and try again.',
      );
      return;
    }
    void sharedServerStatus(bridge, managedProfile).then(
      (status) => {
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
        setManagedStatus(status);
      },
      (error) => {
        if (alive) setManagedStatusError(normalizeCommandError(error).message);
      },
    );
    return () => {
      alive = false;
    };
  }, [bridge, managedProfile, managedStatusAttempt, state, agentReady, world]);

  const commit = useCallback(
    (next: FirstRunCheckpoint): void => {
      checkpointRef.current = next;
      setCheckpoint(next);
      try {
        window.localStorage.setItem(
          FIRST_RUN_CHECKPOINT_KEY,
          encodeFirstRunCheckpoint(next),
        );
      } catch {
        /* unavailable storage */
      }
      onNavigate({ kind: 'first-run', step: next.state, path: next.path });
    },
    [onNavigate],
  );
  const lastReconciledWorld = useRef(world);
  useEffect(() => {
    if (!agentReady || bridge.firstRunFixture) return;
    if (lastReconciledWorld.current === world) return;
    lastReconciledWorld.current = world;
    const reconciled = resolveProvisionedIdentity(
      world,
      reconcileFirstRunCheckpoint(
        checkpoint,
        authoritativeSetupFacts(world, checkpoint),
      ),
    );
    if (reconciled !== checkpoint) commit(reconciled);
  }, [agentReady, bridge.firstRunFixture, checkpoint, commit, world]);
  const send = useCallback(
    (event: Parameters<typeof transitionFirstRun>[1]): void => {
      commit(transitionFirstRun(checkpointRef.current, event));
    },
    [commit],
  );
  const go = useCallback(
    (next: FirstRunStateName): void => {
      setMessage(null);
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
      send({ type: 'go', state: next });
    },
    [checkpoint.backupCommitted, send, state],
  );
  const openExisting = useCallback(
    (from: FirstRunStateName): void => {
      setExistingBack(from);
      go('existing');
    },
    [go],
  );
  const fail = useCallback(
    (
      operation: FirstRunOperation,
      error: unknown,
      report: (message: string) => void = setMessage,
    ): void => {
      const failure = classifyFirstRunFailure(operation, error);
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
        await onRefreshWorld();
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
    [bridge, onRefreshWorld, profile],
  );

  useEffect(() => {
    if (!agentReady) setIdentityLoading(false);
    const generation = identityGeneration;
    return () => {
      generation.current++;
    };
  }, [agentReady]);

  const refreshAccountIdentity = async (
    saved = checkpointRef.current,
  ): Promise<void> => {
    if (!saved.provisionedAccount || !agentReady) return;
    const generation = ++identityGeneration.current;
    setIdentityLoading(true);
    setIdentityError(null);
    try {
      const refreshed = await onRefreshWorld();
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const resolved = resolveProvisionedIdentity(refreshed, saved);
      if (resolved !== saved) commit(resolved);
      else
        setIdentityError(
          'The account details are not available yet. Retry loading them; account setup will not be repeated.',
        );
    } catch (error) {
      if (
        generation !== identityGeneration.current ||
        checkpointRef.current !== saved
      )
        return;
      const typed = normalizeCommandError(error);
      setIdentityError(typed.message);
      if (isAgentReadinessError(typed)) onAgentReadinessFailure?.(typed);
    } finally {
      if (generation === identityGeneration.current) setIdentityLoading(false);
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
  }, []);

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
              setRecoveryAlias((current) => current || resumableAlias);
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

  const editServerAddress = (value: string): void => {
    const input = serverAddressInput.current;
    if (input && document.activeElement === input)
      addressSelection.current = {
        start: input.selectionStart,
        end: input.selectionEnd,
      };
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
      if (state === 'error') send({ type: 'go', state: 'address' });
      return;
    }
    const revision = addressRevision.current;
    setAddressInvalid(false);
    setBusy(true);
    setMessage(null);
    setServerCheckFailure(null);
    try {
      const profileName =
        facts?.profile ??
        `setup-${address
          .toLowerCase()
          .replace(/[^a-z0-9_-]+/g, '-')
          .slice(0, 52)}`;
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
      send({
        type: 'profile-checked',
        address: address.trim(),
        profile: report,
      });
    } catch (error) {
      if (revision !== addressRevision.current) return;
      fail('server-check', error);
      send({ type: 'go', state: 'error' });
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
      setInvite('');
      accountProvisioned(accountAlias);
    } catch (error) {
      fail('account-signup', error);
    } finally {
      setBusy(false);
    }
  };

  const copyGoCandidate = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !goCandidate?.copyable || !recoveryTargetAlias) return;
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, copy: null }));
    try {
      const copied = await bridge.copyGoProfileDevice(
        goCandidate.candidateId,
        profile.profile,
        recoveryTargetAlias,
      );
      if (copied.alias !== recoveryTargetAlias)
        throw new Error('The imported profile belongs to a different account.');
      accountProvisioned(recoveryTargetAlias);
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
      !recoveryTargetAlias
    )
      return;
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, recover: null }));
    try {
      const resumable = pending.find(
        (row) =>
          row.kind === 'account-recovery' &&
          row.alias === recoveryTargetAlias &&
          !row.target,
      );
      if (resumable)
        await bridge.resumeOwnerRecovery(
          profile.profile,
          recoveryTargetAlias,
          recoveryPhrase,
          deviceName.trim(),
        );
      else
        await bridge.recoverOwnerAccount(
          profile.profile,
          recoveryTargetAlias,
          recoveryPhrase,
          deviceName.trim(),
        );
      setRecoveryPhrase('');
      accountProvisioned(recoveryTargetAlias);
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
      !recoveryTargetAlias ||
      !deviceName.trim()
    )
      return;
    const phrase = pairingPhrase;
    if (!resume && !phrase.trim()) return;
    setPairingPhrase('');
    setBusy(true);
    setConnectionErrors((old) => ({ ...old, pair: null }));
    try {
      const provision = resume
        ? await bridge.resumeGoProfilePairing(
            goCandidate.candidateId,
            profile.profile,
            recoveryTargetAlias,
          )
        : await bridge.acceptGoProfilePairing(
            goCandidate.candidateId,
            profile.profile,
            recoveryTargetAlias,
            deviceName.trim(),
            phrase,
          );
      if (provision.alias !== recoveryTargetAlias) {
        throw new Error(
          'The paired device credentials belong to a different account.',
        );
      }
      accountProvisioned(recoveryTargetAlias);
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
      let next = checkpoint;
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
          type: 'go',
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
      let next = checkpoint;
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
    send({ type: 'go', state: 'protect' });
  };

  const commitBackup = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !backupPhrase || !phraseWritten) return;
    if (checkpoint.backupCommitted) {
      setPhraseWritten(false);
      send({ type: 'go', state: 'protect' });
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

  const discover = async (): Promise<void> => {
    if (!agentReady) return;
    if (!profile || !checkpoint.account) return;
    const accountAlias = checkpoint.account.alias;
    setBusy(true);
    setMessage(null);
    try {
      const result = await enqueueProfileWork(bridge, profile.profile, () =>
        bridge.discoverGroups(profile.profile, accountAlias),
      );
      if (result.accountAlias !== checkpoint.account.alias) {
        throw new Error(
          'Group discovery returned data for a different account. Restart FOKS to continue.',
        );
      }
      const match = result.groups.filter(
        (candidate) =>
          candidate.active &&
          (!facts?.groupName || candidate.name === facts.groupName),
      );
      if (match.length !== 1) {
        setMessage(
          facts?.groupName
            ? `Last check: group "${facts.groupName}" is not available.`
            : 'Last check: no matching active group found.',
        );
        return;
      }
      const found = match[0];
      if (!found) return;
      const refreshed = await onRefreshWorld();
      const stores = refreshed.stores.filter(
        (store) =>
          store.kind === 'team' &&
          store.active &&
          store.server === profile.profile &&
          store.account === checkpoint.account?.alias &&
          store.team_id_hex === found.teamIdHex &&
          store.alias === found.alias &&
          store.team_kind === found.kind &&
          (found.kind !== 'named' || store.name === found.name),
      );
      if (stores.length !== 1) {
        setMessage(
          'The group was found, but storage is not ready. Retry after initialization completes.',
        );
        return;
      }
      send({
        type: 'group-discovered',
        group: {
          name: stores[0].name,
          kind: found.kind,
          alias: found.alias,
          teamIdHex: found.teamIdHex,
        },
      });
    } catch (error) {
      fail('group-discovery', error);
    } finally {
      setBusy(false);
    }
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
    if (returning) setExistingBack('local');
    send({
      type: 'managed-profile-selected',
      address: managedStatus.configuredProbe,
      profile: managedReport,
      returning,
    });
  };

  let content: ReactNode;
  if (state === 'identity-pending')
    content = (
      <Pane title="Load account details" header={false}>
        <h1>Your account is connected</h1>
        <p className="lead">
          Account setup completed. FOKS still needs to load the account details
          before continuing. Your progress is saved; setup will not be repeated.
        </p>
        {identityError ? (
          <p className="crit" role="alert">
            {identityError}
          </p>
        ) : null}
        <Button
          variant="primary"
          disabled={identityLoading || !agentReady}
          onClick={() => void refreshAccountIdentity()}
        >
          {identityLoading
            ? 'Loading account details…'
            : 'Retry loading account'}
        </Button>
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
          A private FOKS server is already running on this Mac. Use it to create
          your Personal vault.
        </p>
        <div className="local-server-card">
          <div className="local-server-head">
            <span className="local-server-mark" aria-hidden="true">
              <Icon name="server" />
            </span>
            <span className="local-server-title">
              <b>Local server</b>
              <small>On this Mac</small>
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
      <Pane title="Checking this Mac" header={false}>
        <h1>Looking for existing FOKS accounts</h1>
        <p className="lead">
          Checking for existing accounts from the official FOKS CLI. No changes
          will be made to your data.
        </p>
        <Button disabled>Checking…</Button>
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
          onSelect={setGoCandidate}
        />
        <div className="actions">
          <Button
            variant="primary"
            disabled={!goCandidate}
            onClick={() => {
              if (!goCandidate) return;
              setAddress(goCandidate.serverHint ?? '');
              setRecoveryAlias(
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
          <p className="crit">
            Could not check existing CLI profiles: {goScanError}
          </p>
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
        title={checkpoint.path === 'invited' ? 'Group Server' : 'Server Setup'}
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
          FOKS stores your account, groups, and encrypted vaults on a server.{' '}
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
        {checkpoint.returning ? (
          <p className="hint">
            <b>You already have an account on this server.</b> This Mac will be
            added to your existing account without creating a new one.
          </p>
        ) : null}
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
                'Review the reported error and server address before retrying.'}
            </p>
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
              text="What’s the address of the FOKS server our group is on?"
              onCopy={(value) =>
                void bridge
                  .copyText(value)
                  .then(() => toasts.show('Copied to clipboard.'))
              }
            >
              “What’s the address of the FOKS server our group is on?”
            </CopyBox>
            <p>
              In the next step, you will choose a username and share it with
              them so they can add you to the group.
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
  else if (state === 'checked' || state === 'compare')
    content = (
      <Pane
        title={
          checkpoint.path === 'invited' ? 'Group server' : 'Server details'
        }
        header={false}
        scope="Server verified and pinned. No user data sent."
        foot={
          <Foot back={() => go('address')}>
            <Button
              variant="primary"
              disabled={!profile}
              onClick={() =>
                checkpoint.returning ? openExisting('checked') : go('account')
              }
            >
              Continue
            </Button>
          </Foot>
        }
      >
        <h1>Select a server</h1>
        <p className="lead">
          FOKS synchronizes your account, groups, and encrypted vaults through a
          server.
        </p>
        <Inset className="checked-address">
          <InsetRow
            label="Address"
            action={
              <Button disabled={busy} onClick={() => void checkServer()}>
                Check again
              </Button>
            }
          >
            <input
              value={address}
              placeholder="e.g. foks.app:4430"
              spellCheck={false}
              ref={serverAddressInput}
              aria-label="Server address"
              onChange={(event) => editServerAddress(event.target.value)}
            />
          </InsetRow>
        </Inset>
        <div className="pcard">
          <h3>
            <Icon name="server" /> {profile?.canonicalName} verified{' '}
          </h3>

          <Toggle label="Details" defaultOpen={state === 'compare'}>
            <div className="dbody">
              <div className="facts">
                <div>
                  <span className="k">Lookup name</span>
                  <code>{profile?.lookupName}</code>
                </div>
                <div>
                  <span className="k">Confirmed name</span>
                  <code>{profile?.canonicalName}</code>
                </div>
                <div>
                  <span className="k">Server version</span>
                  <span>{profile?.chain}</span>
                </div>
                <div>
                  <span className="k">Verified</span>
                  <span>
                    {profile
                      ? new Date(profile.epoch * 1000).toLocaleDateString()
                      : '—'}
                  </span>
                </div>
              </div>
              <p className="hint">
                The server certificate was verified on first connection, and its
                host ID is now pinned for future connections.
              </p>
              <Toggle label="Inspect response">
                <pre>{JSON.stringify(profile, null, 1)}</pre>
              </Toggle>
            </div>
          </Toggle>
        </div>
      </Pane>
    );
  else if (state === 'account' && checkpoint.managedLocal)
    content = (
      <Pane
        title="Your account"
        header={false}
        foot={
          <Foot back={() => go(accountBack ?? 'local')}>
            <Button
              variant="primary"
              disabled={
                busy ||
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
              onChange={(event) => setUsername(event.target.value)}
            />
          </label>
          <label className="local-field-row">
            <span>Device name</span>
            <input
              value={deviceName}
              placeholder="Your Mac"
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
        {message ? <p className="crit">{message}</p> : null}
        <button
          className="lnk local-recover-link"
          onClick={() => openExisting('account')}
        >
          Recover an existing account…
        </button>
      </Pane>
    );
  else if (state === 'account')
    content = (
      <Pane
        title="Your account"
        header={false}
        scope="Keys are generated securely on your device."
        wide
        foot={
          <Foot
            back={() =>
              go(accountBack ?? (checkpoint.managedLocal ? 'local' : 'checked'))
            }
          >
            <Button
              variant="primary"
              disabled={
                busy ||
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
          </Foot>
        }
      >
        <h1>Create an account</h1>
        <p className="lead">
          FOKS creates your account keys on this Mac and registers only the
          public keys with the server.
        </p>
        <div className="two account-form">
          <div>
            <SectionLabel>You</SectionLabel>
            <Inset>
              <InsetRow label="Username">
                <input
                  value={username}
                  placeholder="yourname"
                  onChange={(event) => setUsername(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="This Mac’s name">
                <input
                  value={deviceName}
                  placeholder="Your Mac"
                  onChange={(event) => setDeviceName(event.target.value)}
                />
              </InsetRow>
            </Inset>
          </div>
          <div>
            <SectionLabel>Optional</SectionLabel>
            <Inset>
              <InsetRow label="Email (optional)">
                <input
                  value={email}
                  placeholder="you@example.net"
                  onChange={(event) => setEmail(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="Invite (optional)">
                <input
                  value={invite}
                  onChange={(event) => setInvite(event.target.value)}
                />
              </InsetRow>
            </Inset>
          </div>
        </div>
        {profile && !checkpoint.account && (
          <SsoPanel
            key={`${profile.profile}/${accountAlias}`}
            bridge={bridge}
            profile={profile.profile}
            account={accountAlias}
            login={false}
            deviceName={deviceName}
            invite={invite}
            disabled={busy}
            onComplete={() => {
              setInvite('');
              accountProvisioned(accountAlias);
            }}
          />
        )}
        {message ? <p className="crit">{message}</p> : null}
        <button
          className="lnk account-recover-link"
          onClick={() => openExisting('account')}
        >
          Sign in to an existing account
        </button>
      </Pane>
    );
  else if (state === 'existing')
    content = (
      <Pane
        title="Your account"
        header={false}
        scope="Device authorization required"
        wide
        foot={
          <Foot back={() => go(checkpoint.account ? 'protect' : existingBack)}>
            {checkpoint.account ? (
              <Button onClick={() => go('protect')}>Resume protection</Button>
            ) : (
              <Button
                onClick={() => {
                  setAccountBack('existing');
                  go('account');
                }}
              >
                Create a new account
              </Button>
            )}
          </Foot>
        }
      >
        <h1>Add this Mac to your account</h1>
        <p className="lead">
          Your account already exists on {profile?.canonicalName}. Recover it
          with your backup phrase
          {goCandidate ? ' or connect using the official FOKS CLI' : ''}.
        </p>
        <div className="two">
          <div className="pcard">
            <h3>Recover with your backup phrase</h3>
            <p>
              Enter all 17 words from your backup phrase to restore full access
              on this Mac.
            </p>
            <Inset className="recovery-fields">
              {bridge.native ? (
                <InsetRow label="Account alias">
                  <input
                    value={recoveryAlias}
                    onChange={(event) => setRecoveryAlias(event.target.value)}
                  />
                </InsetRow>
              ) : null}
              <InsetRow label="Phrase">
                <input
                  type="password"
                  aria-label="Backup phrase"
                  placeholder="word word word …"
                  value={recoveryPhrase}
                  onChange={(event) => setRecoveryPhrase(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="This Mac’s name">
                <input
                  value={deviceName}
                  placeholder="Your Mac"
                  onChange={(event) => setDeviceName(event.target.value)}
                />
              </InsetRow>
            </Inset>
            <div className="btns">
              <Button
                variant="primary"
                disabled={
                  busy ||
                  !recoveryTargetAlias ||
                  !recoveryPhrase.trim() ||
                  !deviceName.trim()
                }
                onClick={() => void recover()}
              >
                {pending.some(
                  (row) =>
                    row.kind === 'account-recovery' &&
                    row.alias === recoveryTargetAlias,
                )
                  ? 'Resume recovery'
                  : 'Recover'}
              </Button>
            </div>
            {connectionErrors.recover ? (
              <p className="crit" role="alert">
                {connectionErrors.recover}
              </p>
            ) : null}
          </div>
          {goCandidate?.copyable ? (
            <div className="pcard">
              <h3>Copy this Mac’s CLI device</h3>
              <p>
                Advanced: both apps will share the same device credentials.
                macOS may request Keychain access. Revoking the device in either
                client will disable both, and changing your CLI passphrase will
                not update this desktop copy.
              </p>
              <Inset className="recovery-fields">
                <InsetRow label="Account alias">
                  <input
                    aria-label="Copied account alias"
                    value={recoveryAlias}
                    onChange={(event) => setRecoveryAlias(event.target.value)}
                  />
                </InsetRow>
              </Inset>
              <Button
                variant="primary"
                className="copy-device"
                disabled={busy || !recoveryTargetAlias}
                onClick={() => void copyGoCandidate()}
              >
                Copy existing device
              </Button>
              {connectionErrors.copy ? (
                <p className="crit" role="alert">
                  {connectionErrors.copy}
                </p>
              ) : null}
            </div>
          ) : null}
          {goCandidate?.pairable ? (
            <div className="pcard">
              <h3>Use the CLI to approve this as a new device</h3>
              <p>
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
              <p>
                Select the account in the CLI, enter the pairing code below, and
                follow the terminal prompts to complete pairing.
              </p>
              <Inset className="recovery-fields">
                <InsetRow label="Account alias">
                  <input
                    aria-label="Pairing account alias"
                    value={recoveryAlias}
                    onChange={(event) => setRecoveryAlias(event.target.value)}
                  />
                </InsetRow>
                <InsetRow label="This Mac’s name">
                  <input
                    aria-label="Pairing device name"
                    value={deviceName}
                    placeholder="Your Mac"
                    onChange={(event) => setDeviceName(event.target.value)}
                  />
                </InsetRow>
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
                  disabled={
                    busy ||
                    !recoveryTargetAlias ||
                    !deviceName.trim() ||
                    !pairingPhrase.trim()
                  }
                  onClick={() => void acceptPairing(false)}
                >
                  Accept pairing
                </Button>
                <Button
                  disabled={busy || !recoveryTargetAlias || !deviceName.trim()}
                  onClick={() => void acceptPairing(true)}
                >
                  Resume pairing
                </Button>
              </div>
              {connectionErrors.pair ? (
                <p className="crit" role="alert">
                  {connectionErrors.pair}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>
        <p className="hint">This Mac will be added to your account.</p>
      </Pane>
    );
  else if (
    (state === 'protect' || state === 'phrase') &&
    checkpoint.managedLocal
  )
    content = (
      <Pane
        title="Recovery"
        header={false}
        foot={
          <Foot
            back={() => go(checkpoint.returning ? 'existing' : 'account')}
            note={
              <button
                className="lnk"
                onClick={() => void finishLocalProtection(true)}
              >
                Do this later
              </button>
            }
          >
            <Button
              variant="primary"
              disabled={
                busy ||
                (state === 'phrase' && (!backupPhrase || !phraseWritten))
              }
              onClick={() => void finishLocalProtection()}
            >
              Start using FOKS
            </Button>
          </Foot>
        }
      >
        <h1>Set up account recovery</h1>
        <p className="lead">
          Set up a backup phrase now so you can recover your account if this Mac
          is lost.
        </p>
        <div className="local-recovery-card">
          <div className="local-recovery-head">
            <h2>Backup phrase</h2>
            <Chip>Recommended</Chip>
          </div>
          <p>
            Write down these 17 words and keep them somewhere other than this
            Mac. Anyone with them can recover your account.
          </p>
          {state === 'phrase' ? (
            <>
              {backupPhrase ? (
                <div className="words">
                  {backupPhrase.split(/\s+/).map((word, index) => (
                    <div className="word" key={`${index}-${word}`}>
                      <i>{index + 1}</i>
                      {word}
                    </div>
                  ))}
                </div>
              ) : (
                <p>Preparing your phrase…</p>
              )}
              <label className="local-confirm">
                <input
                  type="checkbox"
                  checked={phraseWritten}
                  onChange={(event) => setPhraseWritten(event.target.checked)}
                />
                <span>I have written down all 17 words.</span>
              </label>
              <button
                className="lnk local-quiet-link"
                onClick={collapseLocalBackup}
              >
                Hide recovery phrase
              </button>
            </>
          ) : (
            <Button
              variant="primary"
              disabled={checkpoint.backupCommitted || busy}
              onClick={() => go('phrase')}
            >
              {checkpoint.backupCommitted
                ? 'Recovery phrase saved'
                : 'Show recovery phrase'}
            </Button>
          )}
        </div>
        <div className="local-other-protection">
          <div className="local-quiet-card">
            <b>Passphrase</b>
            <span>Add one later from Settings.</span>
          </div>
        </div>
        {message ? <p className="crit">{message}</p> : null}
      </Pane>
    );
  else if (state === 'local-done')
    content = (
      <Pane
        title="Ready"
        subtitle="Setup complete"
        header={false}
        foot={
          <Foot>
            {!personalAvailable ? (
              <Button
                variant="primary"
                disabled={personalRefreshing}
                onClick={() => {
                  setPersonalRefreshing(true);
                  setPersonalRefreshError(null);
                  void onRefreshWorld()
                    .catch((error) => {
                      const typed = normalizeCommandError(error);
                      setPersonalRefreshError(typed.message);
                      if (isAgentReadinessError(typed))
                        onAgentReadinessFailure?.(typed);
                    })
                    .finally(() => setPersonalRefreshing(false));
                }}
              >
                {personalRefreshing
                  ? 'Loading Personal…'
                  : 'Retry loading Personal'}
              </Button>
            ) : (
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
            )}
          </Foot>
        }
      >
        <div className="local-success">
          <span className="local-success-mark" aria-hidden="true">
            ✓
          </span>
          <h1>
            {personalAvailable
              ? 'Your Personal vault is ready'
              : 'Setup is complete'}
          </h1>
          <p className="lead">
            {personalAvailable
              ? 'Your account is connected to the local server on this Mac.'
              : 'FOKS could not load your Personal vault. Your setup progress is saved. Retry loading the vault to continue.'}
          </p>
          {personalRefreshError ? (
            <p className="crit" role="alert">
              {personalRefreshError}
            </p>
          ) : null}
          <div className="local-vault-preview">
            <div className="local-vault-head">
              <Icon name="vault" />
              <b>Personal</b>
              <code>{profile?.canonicalName}</code>
            </div>
            <div className="local-vault-empty">
              {!personalAvailable
                ? 'Personal vault unavailable'
                : accountItemCount === 0
                  ? 'No items yet'
                  : plural(accountItemCount, 'item')}
            </div>
          </div>
        </div>
      </Pane>
    );
  else if (state === 'protect' || state === 'phrase')
    content = (
      <Pane
        title="Save recovery phrase"
        header={false}
        scope="Configure account recovery"
        wide
        foot={
          <Foot back={() => go('account')}>
            <Button
              variant="primary"
              disabled={busy || passphrase !== confirmation}
              onClick={() => void continueProtection()}
            >
              Continue
            </Button>
          </Foot>
        }
      >
        <h1>Save recovery phrase</h1>
        <p className="lead">
          Only this Mac can recover {checkpoint.account?.username}. Add at least
          one recovery method now. You can manage recovery methods later in
          Settings.
        </p>
        <div className="two">
          <div className="pcard">
            <h3>Backup phrase</h3>
            <p>
              Write down these 17 words to recover your account if every device
              is lost.
            </p>
            <Button
              disabled={busy || (checkpoint.backupCommitted && !backupPhrase)}
              onClick={() => go('phrase')}
            >
              Show my phrase
            </Button>
          </div>
          <div className="pcard">
            <h3>Passphrase</h3>
            <p>
              Protects the keys stored on this Mac with a password. Optional.
            </p>
            <Inset>
              <InsetRow label="Passphrase">
                <input
                  type="password"
                  aria-label="Passphrase"
                  placeholder="••••••••••••"
                  value={passphrase}
                  onChange={(event) => setPassphrase(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="Confirm">
                <input
                  type="password"
                  aria-label="Confirm passphrase"
                  placeholder="••••••••••••"
                  value={confirmation}
                  onChange={(event) => setConfirmation(event.target.value)}
                />
              </InsetRow>
            </Inset>
          </div>
        </div>
        {message ? <p className="crit">{message}</p> : null}
        {state === 'phrase' ? (
          <SheetDialog
            width="wide"
            onClose={() => go('protect')}
            glyph={<Icon name="key" />}
            title="Write these 17 words down"
            subtitle="Keep them somewhere other than this Mac"
            footer={
              <>
                <Button onClick={() => go('protect')}>Not now</Button>
                <Button
                  variant="primary"
                  disabled={!backupPhrase || !phraseWritten || busy}
                  onClick={() => void commitBackup()}
                >
                  Done
                </Button>
              </>
            }
          >
            <>
              <p>
                Anyone with these words can access your account. Store them
                somewhere other than this Mac.
              </p>
              {backupPhrase ? (
                <div className="words">
                  {backupPhrase.split(/\s+/).map((word, index) => (
                    <div className="word" key={`${index}-${word}`}>
                      <i>{index + 1}</i>
                      {word}
                    </div>
                  ))}
                </div>
              ) : (
                <p>Preparing your phrase…</p>
              )}
              <button
                type="button"
                aria-label="I have written these 17 words down"
                className={`check${phraseWritten ? ' on' : ''}`}
                onClick={() => setPhraseWritten((value) => !value)}
              >
                <span className="bx">{phraseWritten ? '✓' : ''}</span>I have
                written these 17 words down
              </button>
            </>
          </SheetDialog>
        ) : null}
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
                <b>{group} will appear under GROUPS</b>
                <span className="hint">
                  Groups appear once membership is confirmed by the server.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="eye" />}>
                <b>Access depends on your group role</b>
                <span className="hint">
                  Roles include <b>Member</b>, <b>Admin</b>, and <b>Owner</b>.
                  You can only access items permitted by your assigned role and
                  visibility level.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="door" />}>
                <b>You can close FOKS anytime</b>
                <span className="hint">
                  Your account and server settings are saved on this Mac. When
                  you reopen FOKS, you can continue setup.
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
                    <>Group not found yet. Only Personal is available.</>
                  ) : null}
                </span>
              </div>
              <p>
                Select <b>Check now</b> to look for pending group invitations.
                FOKS will also check automatically each time you open the app.
              </p>
              <Band label="Group updates">
                <b>Check now</b> checks the server for group memberships linked
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
                  {profile?.canonicalName} and updates your group list.
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
  else if (state === 'checklist-invited' || state === 'checklist-own')
    content = (
      <Pane
        title="Get started"
        subtitle={`${completedFirstRunSteps(checkpoint)} of ${firstRunStepCount(checkpoint)} steps completed`}
        wide
        foot={
          <Foot>
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
          </Foot>
        }
      >
        <p className="lead">
          {completedFirstRunSteps(checkpoint) === firstRunStepCount(checkpoint)
            ? 'Your account is ready. Start using your Personal vault.'
            : 'Completed steps are saved. You can finish account recovery below or start using your Personal vault.'}
        </p>
        <Inset className="checklist">
          <InsetRow label="✓">
            <b>{checkpoint.path === 'invited' ? 'Their server' : 'A server'}</b>
            <span className="hint">
              <code>{profile?.canonicalName}</code> · host ID{' '}
              <code>{profile?.hostId.slice(0, 10)}…</code> · checked and pinned
              on this Mac.
            </span>
          </InsetRow>
          <InsetRow label="✓">
            <b>Your account</b>
            <span className="hint">
              <code>{checkpoint.account?.username}</code> on{' '}
              {profile?.canonicalName} · Device name:{' '}
              <b>{checkpoint.account?.deviceName}</b>.
            </span>
          </InsetRow>
          <InsetRow
            label={
              !(checkpoint.passphraseSet || checkpoint.backupCommitted)
                ? '!'
                : '✓'
            }
            action={
              <Button
                size="sm"
                variant={
                  !(checkpoint.passphraseSet || checkpoint.backupCommitted)
                    ? 'primary'
                    : undefined
                }
                onClick={() => go('protect')}
              >
                {!(checkpoint.passphraseSet || checkpoint.backupCommitted)
                  ? 'Protect now'
                  : 'Review'}
              </Button>
            }
          >
            <b>Save recovery phrase</b>
            <span className="hint">
              {!(checkpoint.passphraseSet || checkpoint.backupCommitted)
                ? `Skipped. If this Mac is lost, you will need a backup phrase, passphrase, or security key to recover your account.`
                : 'Passphrase set · backup phrase saved · security keys can be configured in Settings'}
            </span>
          </InsetRow>
          {checkpoint.path === 'invited' ? (
            <InsetRow
              label={checkpoint.added ? '✓' : '5'}
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
              <Band label="Group discovery">
                Check now looks up groups for this signed-in account.
              </Band>
            </InsetRow>
          ) : null}
        </Inset>
        {!(checkpoint.passphraseSet || checkpoint.backupCommitted) ? (
          <div className="checklist-notice">
            <Notice
              title={`Only this Mac can recover ${checkpoint.account?.username}`}
            >
              Without a backup method, your account cannot be recovered if this
              Mac is lost. You can set up recovery now or continue with setup.
            </Notice>
          </div>
        ) : null}
      </Pane>
    );
  else
    content = (
      <Pane title={group} subtitle={`Group on ${profile?.canonicalName}`} wide>
        <Notice
          title={`Joined ${group}`}
          actions={
            <>
              <Button onClick={() => onNavigate({ kind: 'all' })}>
                Dismiss
              </Button>
              <Button
                variant="primary"
                disabled={!addedStore}
                onClick={() => {
                  if (addedStore)
                    onNavigate({ kind: 'store', ref: addedStore });
                }}
              >
                Open group
              </Button>
            </>
          }
        >
          <p>
            You have been added to this group as{' '}
            <code>{checkpoint.account?.username}</code>. You can now access the
            items listed below.
          </p>
        </Notice>
        <div className="hdr">
          <span />
          <span>Name</span>
          <span>Access</span>
          <span>Version</span>
          <span />
        </div>
        {world.items
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
                <Chip>{readableBy(world, item).label}</Chip>
              </span>
              <span className="n">v{item.version}</span>
              <span />
            </div>
          ))}
      </Pane>
    );

  const appMode = ['added', 'checklist-invited', 'checklist-own'].includes(
    state,
  );
  const canCancelSetup =
    world.accounts.length > 0 ||
    goCandidates.length > 0 ||
    world.stores.some((store) => store.kind === 'account');
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
          world={world}
          checkpoint={checkpoint}
          groupName={checkpoint.group?.name ?? group}
          location={location}
          onNavigate={onNavigate}
          onReenter={() => send({ type: 'reenter' })}
        />
      ) : (
        <SetupSidebar
          checkpoint={checkpoint}
          pendingPath={pendingPath}
          onAnotherServer={
            !busy &&
            checkpoint.managedLocal &&
            !checkpoint.account &&
            !checkpoint.provisionedAccount
              ? () => send({ type: 'choose', path: 'own' })
              : undefined
          }
          onRecoverAccount={
            !busy &&
            checkpoint.managedLocal &&
            !checkpoint.account &&
            !checkpoint.provisionedAccount
              ? () => selectManagedProfile(true)
              : undefined
          }
          recoverEnabled={Boolean(managedReport)}
          onCancel={
            !busy && canCancelSetup
              ? () => onNavigate({ kind: 'all' })
              : undefined
          }
        />
      )}
      <main
        className="main first-run-main"
        inert={agentReady && busy}
        aria-busy={agentReady && busy}
      >
        {readinessBlocker ?? content}
      </main>
      {state === 'added' ? (
        <AddedDetails world={world} storeId={addedStore} />
      ) : null}
    </>
  );
}
