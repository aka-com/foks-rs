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
  SheetDialog,
  Toggle,
} from '../components';
import type { FilterKind } from '../components';
import type {
  Bridge,
  CheckedProfileResponse,
  GoProfileCandidate,
  GoProfileDiscovery,
  PendingOperation,
  ServerStatusSnapshot,
} from '../bridge';
import {
  enqueueProfileWork,
  normalizeCommandError,
  sharedServerStatus,
} from '../bridge';
import {
  FIRST_RUN_CHECKPOINT_KEY,
  completedFirstRunSteps,
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  initialFirstRun,
  isFirstRunState,
  transitionFirstRun,
} from '../first-run-state';
import type {
  FirstRunCheckpoint,
  FirstRunGroupKind,
  FirstRunPath,
  FirstRunStateName,
} from '../first-run-state';
import type { FoksIconName } from '../icons';
import type { Location } from '../location';
import { NavRow, Sidebar } from '../shell/sidebar';
import { formatRole, kindOf, parseRole, plural, storeReadable } from '../model';
import type { RoleWire, TeamStore, World } from '../model';
import { PageHeader } from '../shell/page-header';
import { GoProfileChooser } from './go-profile-chooser';
import { readableBy } from './scope';
import { useToast } from '/kit/toasts';

const PERSONAL_FIXED =
  'Your Personal vault belongs only to your account and has no members. Put items in a group to share them.';

const stepOf = (state: FirstRunStateName): number => {
  if (state === 'boot') return 0;
  if (state === 'local') return 0;
  if (state === 'who') return 1;
  if (['address', 'no-address', 'checked', 'compare', 'error'].includes(state))
    return 2;
  if (state === 'account' || state === 'existing') return 3;
  if (state === 'protect' || state === 'phrase') return 4;
  if (state === 'waiting' || state === 'create-group') return 5;
  return 6;
};

const localStepOf = (state: FirstRunStateName): number => {
  if (state === 'local') return 0;
  if (state === 'account' || state === 'existing') return 1;
  if (state === 'protect' || state === 'phrase') return 2;
  return 3;
};

/** Apple's Computer Name is "{Owner}'s MacBook Pro". Without that string, use the product family. */
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
  if (state === 'boot') return { ...next, initialized: false };
  next = { ...next, initialized: true };
  if (state === 'error' && facts) next = { ...next, serverAddress: facts.typo };
  if (
    (stepOf(state) >= 3 || state === 'checked' || state === 'compare') &&
    facts
  ) {
    next = { ...next, profile: facts.report, serverAddress: facts.server };
  }
  if (stepOf(state) >= 4 && facts) {
    next = {
      ...next,
      account: {
        alias: facts.accountAlias,
        username: facts.username,
        deviceName: facts.deviceName,
      },
    };
  }
  if (stepOf(state) >= 5 && state !== 'phrase')
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
  if (state === 'done')
    next = {
      ...next,
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
      groupSkipped: true,
    };
  return next;
}

function initialCheckpoint(
  bridge: Bridge,
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
  if (automaticEntry && saved) return saved;
  // A phrase URL can remain in browser history, but the prepared phrase
  // itself never survives reload. Resume at Protect and require a fresh
  // explicit reveal.
  const state =
    location.step === 'phrase'
      ? 'protect'
      : (location.step as FirstRunStateName);
  if (
    !saved ||
    (location.path !== undefined && location.path !== saved.path) ||
    (state === 'who' && saved.state !== 'who')
  )
    return initialFirstRun(path, state);
  return { ...saved, path: location.path ?? saved.path, state };
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
  /**
   * What the reader has picked on the `who` screen but not yet confirmed.
   * The checkpoint's path defaults to `invited` before anyone chooses, so
   * without this the sidebar would assert a path the reader has not taken.
   */
  pendingPath?: FirstRunPath | null;
  onCancel?: () => void;
  onAnotherServer?: () => void;
  onRecoverAccount?: () => void;
  recoverEnabled?: boolean;
}): ReactNode {
  if (checkpoint.managedLocal) {
    const current = localStepOf(checkpoint.state);
    const labels = ['Local server', 'Create your account', 'Recovery'];
    return (
      <nav className="side setup-side" aria-label="Setting up">
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
        {onAnotherServer || onRecoverAccount || onCancel ? (
          <div className="foot">
            {onAnotherServer ? (
              <button type="button" className="nav" onClick={onAnotherServer}>
                <Icon name="server" />
                <span className="t">Use another server</span>
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
                <span className="t">Cancel setup</span>
              </button>
            ) : null}
          </div>
        ) : null}
      </nav>
    );
  }
  const current = stepOf(checkpoint.state);
  // On the `who` screen the path is whatever the reader has selected, which
  // may be nothing yet. Step 6 is the one that reads differently per path;
  // until there is a path it says the neutral thing and is drawn pending,
  // rather than naming a route nobody has chosen.
  const choosing = checkpoint.state === 'who';
  const path = choosing ? (pendingPath ?? null) : checkpoint.path;
  const labels = [
    'Preparing this Mac',
    'How are you joining?',
    'Select a server',
    'Create your account',
    'Save recovery phrase',
    path === null
      ? 'Group'
      : path === 'invited'
        ? 'Wait to be added'
        : 'Create a group',
    'You’re in',
  ];
  const pendingSteps = path === null ? new Set([5]) : new Set<number>();
  return (
    <nav className="side setup-side" aria-label="Setting up">
      <div className="setup-steps">
        {labels.map((label, index) => (
          <div
            key={label}
            className={`setup-step${index === current ? ' on' : ''}${index < current ? ' done' : ''}${pendingSteps.has(index) ? ' pending' : ''}`}
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
            <span className="t">Cancel setup</span>
          </button>
        </div>
      ) : null}
    </nav>
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
  const left = completedFirstRunSteps(checkpoint);
  // The vault and the group are real by the time this sidebar is on screen,
  // so the shared sidebar lists them from the world rather than from a
  // preview that could disagree with it. What first run adds is its own
  // progress, which is what the `status` slot is for.
  const status = (
    <>
      <SectionLabel as="side">Status</SectionLabel>
      {left < 5 ? (
        <NavRow
          active
          glyph={<Icon name="flag" />}
          name="Get started"
          tail={<span className="badge">{left} of 5</span>}
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
      {checkpoint.path === 'invited' && !checkpoint.added ? (
        <p className="side-note">
          {groupName} isn’t listed yet. A group appears under GROUPS when this
          Mac asks the server for it; nothing is pushed here.
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
            {kind} in {store?.name ?? 'this group'}
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
            — locked —
          </InsetRow>
        </Inset>
        <p className="pfn">
          Your Member role determines whether this exact version can be opened.
        </p>
        <SectionLabel>Info</SectionLabel>
        <div className="meta">
          <b>Path</b>
          <code>{item.path}</code>
          <b>Kind</b>
          <span>{kind}</span>
          <b>Version</b>
          <span>{item.version}</span>
          <b>Size</b>
          <span>{item.size} bytes</span>
          <b>Read role</b>
          <Chip>{roleText(item.read)}</Chip>
          <b>Write role</b>
          <Chip>{roleText(item.write)}</Chip>
        </div>
        <SectionLabel>Sharing</SectionLabel>
        <p>
          Everyone in {store?.name ?? 'this group'} at the item’s read role or
          above can read it. These facts were learned from the current roster.
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
          ‹ Back
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
  /** Only drawn when `header` is true. */
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

/**
 * The `who` screen's fork: setting up alone, or being added to someone's
 * group. Two options rather than three — "I already use FOKS on another device"
 * is a returning user with no account to create, so it competed with the real
 * choice and now sits beside Continue as a link.
 *
 * Each option carries exactly one fact — what you need before you start —
 * because that is the only thing that distinguishes them at this point. The
 * rest is deferred to `WhatHappensNext`, which fills in for the chosen path.
 */
const JOINING_OPTIONS: readonly {
  path: FirstRunPath;
  icon: FoksIconName;
  title: string;
  detail: string;
  need: string;
}[] = [
  {
    path: 'own',
    icon: 'person',
    title: 'I’m starting on my own',
    detail: 'For yourself, or to start a group that others will join.',
    need: 'You’ll need a server address',
  },
  {
    path: 'invited',
    icon: 'people',
    title: 'Someone invited me to their group',
    detail: 'They said something like “install this and I’ll add you”.',
    need: 'You’ll need their server address',
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
  // A radiogroup is one tab stop; the arrows move within it. Nothing is
  // checked at first, so the group is entered at the first option.
  const move = (from: number, step: number): void => {
    const next =
      (from + step + JOINING_OPTIONS.length) % JOINING_OPTIONS.length;
    onChange(JOINING_OPTIONS[next].path);
    refs.current[next]?.focus();
  };
  return (
    <div className="opts" role="radiogroup" aria-label="How are you joining">
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
            <span className="need">
              <Icon name="server" />
              {option.need}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** The three steps the chosen path actually leads through. */
const NEXT_STEPS: Readonly<Record<FirstRunPath, readonly ReactNode[]>> = {
  invited: [
    <>
      Enter <b>their server address</b> and check it is the right one.
    </>,
    <>
      Pick a <b>username</b> and save your recovery phrase.
    </>,
    <>
      <b>Wait to be added.</b> Joining a server requires admin approval.
    </>,
  ],
  own: [
    <>
      Enter a <b>server address</b>: yours, or one you were given.
    </>,
    <>
      Pick a <b>username</b> and save your recovery phrase.
    </>,
    <>
      <b>Create a group</b> for others to join, or skip it and keep things
      personal.
    </>,
  ],
};

function WhatHappensNext({ path }: { path: FirstRunPath }): ReactNode {
  return (
    <section className="next" aria-live="polite">
      <h3>What happens next</h3>
      <ol>
        {NEXT_STEPS[path].map((step, index) => (
          <li key={index}>
            <span>{step}</span>
          </li>
        ))}
      </ol>
    </section>
  );
}

function GroupKindChoice({
  value,
  onChange,
  server,
}: {
  value: FirstRunGroupKind;
  onChange: (value: FirstRunGroupKind) => void;
  server: string;
}): ReactNode {
  return (
    <RadioGroup label="Group kind">
      {(['named', 'adhoc'] as const).map((kind) => (
        <RadioCard
          key={kind}
          selected={value === kind}
          onSelect={() => onChange(kind)}
          title={kind === 'named' ? 'Named' : 'Ad-hoc'}
          detail={
            kind === 'named'
              ? `Has a name others on ${server} can look up, and can be added to other groups.`
              : 'Known only by its id — for a quick group that never needs to be found by name.'
          }
        />
      ))}
    </RadioGroup>
  );
}

export interface FirstRunExperienceProps {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'first-run' }>;
  onNavigate: (location: Location) => void;
  onRefreshWorld: () => Promise<World>;
  concealSignal: number;
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
  automaticEntry = false,
  managedProfile,
}: FirstRunExperienceProps): ReactNode {
  const toasts = useToast();
  const [checkpoint, setCheckpoint] = useState(() =>
    initialCheckpoint(bridge, location, automaticEntry),
  );
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
  const goScanStarted = useRef(false);
  const [recoveryAlias, setRecoveryAlias] = useState(
    () => checkpoint.account?.alias ?? facts?.accountAlias ?? '',
  );
  const [backupPhrase, setBackupPhrase] = useState<string | null>(null);
  const [phraseWritten, setPhraseWritten] = useState(false);
  // The `who` screen's selection, before Continue commits it. It is screen
  // state, not checkpoint state: nothing has been chosen until Continue.
  const [pendingPath, setPendingPath] = useState<FirstRunPath | null>(null);
  const [groupName, setGroupName] = useState(
    () => checkpoint.group?.name ?? facts?.groupName ?? 'Household',
  );
  const [groupKind, setGroupKind] = useState<FirstRunGroupKind>(
    () => checkpoint.group?.kind ?? 'named',
  );
  const [pending, setPending] = useState<PendingOperation[]>([]);
  const pendingRef = useRef<PendingOperation[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [managedStatus, setManagedStatus] =
    useState<ServerStatusSnapshot | null>(null);
  const [managedStatusError, setManagedStatusError] = useState<string | null>(
    null,
  );
  const [managedStatusAttempt, setManagedStatusAttempt] = useState(0);
  const automaticEntryPending = useRef(automaticEntry);
  const initializationPromise = useRef<Promise<void> | null>(null);
  const [initializationAttempt, setInitializationAttempt] = useState(0);
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
      checkpoint.state !== 'who' ||
      !bridge.native ||
      goChooserDismissed ||
      goScanStarted.current
    )
      return;
    goScanStarted.current = true;
    let alive = true;
    setGoScanError(null);
    void bridge
      .discoverGoProfiles()
      .then((discovery) => {
        if (alive) setGoDiscovery(discovery);
      })
      .catch((error) => {
        if (alive) setGoScanError(normalizeCommandError(error).message);
      });
    return () => {
      alive = false;
    };
  }, [bridge, checkpoint.state, goChooserDismissed]);

  const state = checkpoint.state;
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
  const admin = facts?.admin ?? 'the group admin';
  const adminShort = admin.split('.')[0];
  const group = checkpoint.group?.name ?? facts?.groupName ?? 'the group';
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
          store.team_id_hex === checkpoint.group.teamIdHex &&
          (checkpoint.path !== 'own' ||
            store.account === checkpoint.account?.alias),
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
  const accountItemCount = accountStore
    ? world.items.filter((item) => item.store === accountStore).length
    : 0;

  useEffect(() => {
    if (state !== 'local' || !managedProfile) return;
    setManagedStatus(null);
    let alive = true;
    setManagedStatusError(null);
    if (
      !world.servers.some(
        (server) => server.id === managedProfile && server.state === 'ok',
      )
    ) {
      setManagedStatusError('The managed local server is not ready.');
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
          setManagedStatusError('The managed local server is not ready.');
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
  }, [bridge, managedProfile, managedStatusAttempt, state, world.servers]);

  const commit = useCallback(
    (next: FirstRunCheckpoint): void => {
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
  const send = useCallback(
    (event: Parameters<typeof transitionFirstRun>[1]): void => {
      commit(transitionFirstRun(checkpoint, event));
    },
    [checkpoint, commit],
  );
  const sendRef = useRef(send);
  sendRef.current = send;
  const go = useCallback(
    (next: FirstRunStateName): void => {
      setMessage(null);
      setRecoveryPhrase('');
      setPairingPhrase('');
      setInvite('');
      if (next !== 'protect' && next !== 'phrase') {
        setPassphrase('');
        setConfirmation('');
      }
      if (state === 'phrase' && next !== 'phrase') {
        backupPreparation.current = null;
        setBackupPhrase(null);
        setPhraseWritten(false);
      }
      send({ type: 'go', state: next });
    },
    [send, state],
  );
  const openExisting = useCallback(
    (from: FirstRunStateName): void => {
      setExistingBack(from);
      go('existing');
    },
    [go],
  );
  const fail = useCallback(
    (error: unknown): void => setMessage(normalizeCommandError(error).message),
    [],
  );

  const refreshAccountIdentity = async (alias: string) => {
    if (!profile)
      throw new Error('Check the server before loading an account.');
    const refreshed = await onRefreshWorld();
    const matches = refreshed.accounts.filter(
      (candidate) =>
        candidate.server === profile.profile && candidate.alias === alias,
    );
    if (matches.length !== 1) {
      throw new Error(
        'The account operation finished, but its authenticated identity is not available after refresh. Reopen FOKS before continuing.',
      );
    }
    return matches[0];
  };

  const clearSecrets = useCallback((): void => {
    setInvite('');
    setPassphrase('');
    setConfirmation('');
    setRecoveryPhrase('');
    setPairingPhrase('');
    backupPreparation.current = null;
    setBackupPhrase(null);
    setPhraseWritten(false);
  }, []);

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
    if (!profile) {
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
        if (alive) fail(error);
      },
    );
    return () => {
      alive = false;
    };
  }, [bridge, checkpoint.returning, fail, profile]);

  useEffect(() => {
    if (state !== 'boot' || checkpoint.initialized || !bridge.native) return;
    let alive = true;
    const attempt =
      initializationPromise.current ??
      (initializationPromise.current = bridge
        .initializeClientState()
        .then(() => undefined));
    void attempt.then(
      () => {
        void onRefreshWorld().then(
          (refreshed) => {
            const localReady = Boolean(
              managedProfile &&
              refreshed.servers.some(
                (server) =>
                  server.id === managedProfile && server.state === 'ok',
              ),
            );
            if (alive)
              sendRef.current({ type: 'initialize', managedLocal: localReady });
          },
          (error) => {
            if (alive) fail(error);
          },
        );
      },
      (error) => {
        initializationPromise.current = null;
        if (alive) fail(error);
      },
    );
    return () => {
      alive = false;
    };
  }, [
    bridge,
    checkpoint.initialized,
    fail,
    initializationAttempt,
    managedProfile,
    onRefreshWorld,
    state,
  ]);

  useEffect(() => {
    if (state !== 'phrase' || backupPhrase || !profile || !accountAlias) return;
    let alive = true;
    setBusy(true);
    const key = `${profile.profile}\u0000${accountAlias}`;
    const attempt =
      backupPreparation.current?.key === key
        ? backupPreparation.current.promise
        : bridge.prepareOwnerBackup(profile.profile, accountAlias, 'paper');
    backupPreparation.current = { key, promise: attempt };
    void attempt.then(
      (result) => {
        if (alive) {
          setBackupPhrase(result.phrase);
          setBusy(false);
        }
      },
      (error) => {
        if (!alive) return;
        backupPreparation.current = null;
        setBusy(false);
        go('protect');
        fail(error);
      },
    );
    return () => {
      alive = false;
    };
  }, [accountAlias, backupPhrase, bridge, fail, go, profile, state]);

  const checkServer = async (): Promise<void> => {
    if (!address.trim()) {
      setAddressInvalid(true);
      if (state === 'error') send({ type: 'go', state: 'address' });
      return;
    }
    setAddressInvalid(false);
    setBusy(true);
    setMessage(null);
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
      send({
        type: 'profile-checked',
        address: address.trim(),
        profile: report,
      });
    } catch (error) {
      fail(error);
      send({ type: 'go', state: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const createAccount = async (): Promise<void> => {
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
      const identity = await refreshAccountIdentity(accountAlias);
      send({
        type: 'account-complete',
        alias: accountAlias,
        username: identity.username,
        deviceName: deviceName.trim(),
      });
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const copyGoCandidate = async (): Promise<void> => {
    if (!profile || !goCandidate?.copyable || !recoveryTargetAlias) return;
    setBusy(true);
    setMessage(null);
    try {
      const copied = await bridge.copyGoProfileDevice(
        goCandidate.candidateId,
        profile.profile,
        recoveryTargetAlias,
      );
      if (copied.alias !== recoveryTargetAlias)
        throw new Error('Device copy returned a different account.');
      const identity = await refreshAccountIdentity(recoveryTargetAlias);
      send({
        type: 'account-complete',
        alias: recoveryTargetAlias,
        username: identity.username,
        deviceName: deviceName.trim(),
      });
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const recover = async (): Promise<void> => {
    if (
      !profile ||
      !recoveryPhrase.trim() ||
      !deviceName.trim() ||
      !recoveryTargetAlias
    )
      return;
    setBusy(true);
    setMessage(null);
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
      const identity = await refreshAccountIdentity(recoveryTargetAlias);
      send({
        type: 'account-complete',
        alias: recoveryTargetAlias,
        username: identity.username,
        deviceName: deviceName.trim(),
      });
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const acceptPairing = async (resume: boolean): Promise<void> => {
    if (!profile || !recoveryTargetAlias || !deviceName.trim()) return;
    const phrase = pairingPhrase;
    if (!resume && !phrase.trim()) return;
    setPairingPhrase('');
    setBusy(true);
    setMessage(null);
    try {
      const provision = resume
        ? goCandidate
          ? await bridge.resumeGoProfilePairing(
              goCandidate.candidateId,
              profile.profile,
              recoveryTargetAlias,
            )
          : await bridge.resumeDevicePairingAcceptance(
              profile.profile,
              recoveryTargetAlias,
            )
        : goCandidate
          ? await bridge.acceptGoProfilePairing(
              goCandidate.candidateId,
              profile.profile,
              recoveryTargetAlias,
              deviceName.trim(),
              phrase,
            )
          : await bridge.acceptDevicePairing(
              profile.profile,
              recoveryTargetAlias,
              deviceName.trim(),
              phrase,
            );
      if (provision.alias !== recoveryTargetAlias) {
        throw new Error('Device pairing returned a different account.');
      }
      const identity = await refreshAccountIdentity(recoveryTargetAlias);
      send({
        type: 'account-complete',
        alias: recoveryTargetAlias,
        username: identity.username,
        deviceName: deviceName.trim(),
      });
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const continueProtection = async (): Promise<void> => {
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
          state: checkpoint.path === 'invited' ? 'waiting' : 'create-group',
        }),
      );
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const finishLocalProtection = async (skip = false): Promise<void> => {
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
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const collapseLocalBackup = (): void => {
    setMessage(null);
    send({ type: 'go', state: 'protect' });
  };

  const commitBackup = async (): Promise<void> => {
    if (!profile || !backupPhrase || !phraseWritten) return;
    setBusy(true);
    try {
      await bridge.commitOwnerBackup(
        profile.profile,
        accountAlias,
        'paper',
        backupPhrase,
      );
      setBackupPhrase(null);
      backupPreparation.current = null;
      setPhraseWritten(false);
      send({ type: 'backup-committed' });
    } catch (error) {
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const discover = async (): Promise<void> => {
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
          'The authenticated discovery response belongs to a different account. Reopen FOKS before continuing.',
        );
      }
      const match = result.groups.filter(
        (candidate) =>
          candidate.active &&
          (!facts?.groupName || candidate.name === facts.groupName),
      );
      if (match.length !== 1) {
        setMessage(
          `Checked just now — ${facts?.groupName ?? 'a single active group'} is not listed unambiguously yet.`,
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
          'The authenticated group was found, but its refreshed store is not available yet. Check again.',
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
      fail(error);
    } finally {
      setBusy(false);
    }
  };

  const createGroup = async (): Promise<void> => {
    if (!profile || !checkpoint.account || !groupName.trim()) return;
    setBusy(true);
    setMessage(null);
    try {
      // list_accounts is bound to the catalog generation retained by Rust.
      // Establish that generation first; racing these calls can fail closed.
      const catalog = await bridge.listCatalog();
      const accounts = await bridge.listAccounts();
      const identity = accounts.find(
        (candidate) =>
          candidate.server === profile.profile &&
          candidate.alias === checkpoint.account?.alias,
      );
      const accountStore =
        identity &&
        catalog.stores.find(
          (store) => store.id === identity.store && store.kind === 'account',
        );
      if (!accountStore)
        throw new Error(
          'Your authenticated account store is not available yet. Reopen this step after account setup finishes.',
        );
      const alias = groupName
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9_-]+/g, '-')
        .replace(/^-+|-+$/g, '');
      if (!alias)
        throw new Error('Enter a group name containing letters or numbers.');
      await bridge.createGroup({
        accountStoreId: accountStore.id,
        teamAlias: alias,
        name: groupName.trim(),
        kind: groupKind,
      });
      const refreshed = await onRefreshWorld();
      const expectedPrefix = groupKind === 'named' ? '03' : '14';
      const matches = refreshed.stores.filter(
        (store): store is TeamStore =>
          store.kind === 'team' &&
          store.active === true &&
          store.server === profile.profile &&
          store.account === identity.alias &&
          store.alias === alias &&
          store.name === groupName.trim() &&
          store.team_kind === groupKind &&
          typeof store.team_id_hex === 'string' &&
          new RegExp(`^${expectedPrefix}[0-9a-f]{64}$`).test(store.team_id_hex),
      );
      if (matches.length !== 1 || !matches[0]?.team_id_hex) {
        throw new Error(
          'The group operation finished, but its authenticated active store is not available after refresh. Reopen FOKS before continuing.',
        );
      }
      const created = matches[0];
      send({
        type: 'group-complete',
        group: {
          name: created.name,
          kind: created.team_kind,
          alias: created.alias,
          teamIdHex: created.team_id_hex,
        },
      });
    } catch (error) {
      fail(error);
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
    if (!managedReport || !managedStatus) return;
    if (returning) setExistingBack('local');
    send({
      type: 'managed-profile-selected',
      address: managedStatus.configuredProbe,
      profile: managedReport,
      returning,
    });
  };

  let content: ReactNode;
  if (state === 'boot')
    content = (
      <Pane
        title="Preparing this Mac"
        scope="Automatic — the same on both paths"
        header={false}
        foot={
          <Foot>
            {message ? (
              <Button
                variant="primary"
                onClick={() => {
                  setMessage(null);
                  setInitializationAttempt((attempt) => attempt + 1);
                }}
              >
                Try again
              </Button>
            ) : checkpoint.initialized ? (
              <Button variant="primary" onClick={() => go('who')}>
                Continue
              </Button>
            ) : (
              <Button variant="primary" disabled>
                Continue
              </Button>
            )}
          </Foot>
        }
      >
        <h1>Preparing this Mac</h1>
        <p className="lead">
          {checkpoint.initialized ? (
            <>
              The local <b>foks-agent</b> and encrypted store for your keys are
              ready.
            </>
          ) : (
            <>
              Starting a local <b>foks-agent</b> and initializing an encrypted
              store for your keys.
            </>
          )}
        </p>
        <Inset className="checklist">
          <InsetRow label="✓">
            <b>Starting macOS private helper</b>
            <span className="hint">
              Runs only on this Mac and bound to this window.
            </span>
          </InsetRow>
          <InsetRow label={checkpoint.initialized ? '✓' : '◌'}>
            <b>Creating encrypted vault</b>
            <span className="hint">
              Your keys will be stored in an end-to-end encrypted vault.
            </span>
          </InsetRow>
          <InsetRow label={checkpoint.initialized ? '✓' : '3'}>
            <b>Ready</b>
            <span className="hint">Continue to the next step.</span>
          </InsetRow>
        </Inset>
        {message ? <p className="crit">{message}</p> : null}
        <details className="dd">
          <summary>Details</summary>
          <p>
            Agent status reports Bootstrap then Ready. Initialisation creates
            the native credential and rollback boundary once per Mac.
          </p>
        </details>
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
          <SectionLabel>Other ways to begin</SectionLabel>
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
    bridge.native &&
    !goChooserDismissed &&
    !goDiscovery &&
    !goScanError
  )
    content = (
      <Pane title="Checking this Mac" header={false}>
        <h1>Looking for existing FOKS accounts</h1>
        <p className="lead">
          Checking the official FOKS client’s standard local profile store.
          Nothing is changed or unlocked.
        </p>
        <Button disabled>Checking…</Button>
      </Pane>
    );
  else if (state === 'who' && !goChooserDismissed && goCandidates.length > 0)
    content = (
      <Pane title="FOKS is already set up" header={false} wide>
        <h1>FOKS is already set up on this Mac</h1>
        <p className="lead">
          Choose an account from the official FOKS command-line client. The next
          step adds this desktop as a separate device or copies the existing
          device with your approval.
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
            Connect selected account
          </Button>
          <Button
            onClick={() => {
              setGoCandidate(null);
              setGoChooserDismissed(true);
            }}
          >
            Set up another account
          </Button>
        </div>
      </Pane>
    );
  else if (state === 'who')
    content = (
      <Pane title="How are you joining?" header={false} wide>
        <h1>How are you joining?</h1>
        <p className="lead">
          Pick the one that fits. Everything else is figured out in the next few
          steps.
        </p>
        {goScanError ? (
          <p className="crit">
            Existing CLI profiles could not be inspected: {goScanError}
          </p>
        ) : null}
        <JoiningChoice value={pendingPath} onChange={setPendingPath} />
        {pendingPath ? <WhatHappensNext path={pendingPath} /> : null}
        <div className="actions">
          <Button
            variant="primary"
            disabled={!pendingPath}
            onClick={() => {
              if (pendingPath) send({ type: 'choose', path: pendingPath });
            }}
          >
            Continue
          </Button>
          <span className="alt">
            Already use FOKS on another device?{' '}
            <button
              type="button"
              className="lnk"
              onClick={() =>
                send({ type: 'choose', path: 'own', returning: true })
              }
            >
              Add this as a secondary device
            </button>
          </span>
        </div>
      </Pane>
    );
  else if (['address', 'no-address', 'error'].includes(state))
    content = (
      <Pane
        title={checkpoint.path === 'invited' ? 'Their server' : 'A server'}
        header={false}
        scope={
          state === 'no-address' && checkpoint.path === 'invited'
            ? 'Where the address comes from'
            : undefined
        }
        foot={
          <Foot back={() => go('who')}>
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => void checkServer()}
            >
              {state === 'error' ? 'Check again' : 'Check the server'}
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
          Everything in FOKS lives on a server — a machine someone runs, where
          your account, your groups and their encrypted stores are kept.{' '}
          {checkpoint.path === 'invited'
            ? `Enter the server address from ${adminShort}.`
            : null}
        </p>
        <SectionLabel>Server</SectionLabel>
        <Inset
          className={state === 'error' || addressInvalid ? 'err' : undefined}
        >
          <label className="fr server-address-row">
            <span className="k">Address</span>
            <input
              aria-label="Server address"
              value={address}
              placeholder="e.g. foks.example.net, localhost, etc."
              onChange={(event) => {
                setAddress(event.target.value);
                if (addressInvalid) setAddressInvalid(false);
              }}
            />
          </label>
        </Inset>
        {checkpoint.returning ? (
          <p className="hint">
            <b>You already have an account on this server.</b> Nothing new is
            registered: after the check, this Mac is added to the account you
            already have.
          </p>
        ) : null}
        {state === 'error' && !addressInvalid ? (
          <div className="crit">
            <b>
              {message ??
                `${address || 'The empty address'} did not answer, so nothing was saved`}
            </b>
            FOKS could not connect to this address. Check the address and your
            network connection, then try again. No changes were saved.
          </div>
        ) : null}
        {checkpoint.path === 'invited' && state === 'no-address' ? (
          <div className="pcard">
            <h3>{`Ask ${adminShort} this`}</h3>
            <p>
              Any way you normally talk to them. It is the only thing you need
              from them right now.
            </p>
            <CopyBox
              text="What’s the address of the FOKS server our group is on?"
              onCopy={(value) =>
                void bridge
                  .copyText(value)
                  .then(() => toasts.show('Sentence copied.'))
              }
            >
              “What’s the address of the FOKS server our group is on?”
            </CopyBox>
            <p>
              Your username is next — you choose it here and send it to them
              after. They add you from their side; you never need a code or a
              link.
            </p>
          </div>
        ) : checkpoint.path === 'invited' ? (
          <p className="hint">
            <button className="lnk" onClick={() => go('no-address')}>
              They sent nothing?
            </button>
          </p>
        ) : null}
      </Pane>
    );
  else if (state === 'checked' || state === 'compare')
    content = (
      <Pane
        title={checkpoint.path === 'invited' ? 'Their server' : 'A server'}
        header={false}
        scope="Checked and pinned — nothing about you sent yet"
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
          Everything in FOKS lives on a server — a machine someone runs, where
          your account, your groups and their encrypted stores are kept.
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
              placeholder="e.g. foks.example.net, localhost, etc."
              spellCheck={false}
              onChange={(event) => setAddress(event.target.value)}
            />
          </InsetRow>
        </Inset>
        <div className="pcard">
          <h3>
            <Icon name="server" /> {profile?.canonicalName} answered{' '}
            <Chip>Pinned on this Mac</Chip>
          </h3>
          <p>
            Saved as <b>{profile?.canonicalName}</b>. The host ID is pinned on
            this Mac.
          </p>
          <details className="dd" open={state === 'compare'}>
            <summary>Details</summary>
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
                  <span className="k">Chain sequence</span>
                  <span>{profile?.chain}</span>
                </div>
                <div>
                  <span className="k">Merkle epoch</span>
                  <span>{profile?.epoch.toLocaleString()}</span>
                </div>
                <div className="wide">
                  <span className="k">Host ID</span>
                  <code>{profile?.hostId.match(/.{1,4}/g)?.join(' ')}</code>
                </div>
              </div>
              <p className="hint">
                These values came from this check. The certificate established
                the first connection; future checks use the pinned host ID.
              </p>
              <Toggle label="Inspect response">
                <pre>{JSON.stringify(profile, null, 1)}</pre>
              </Toggle>
            </div>
          </details>
        </div>
      </Pane>
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
          Choose how you appear on this server and name this Mac.
        </p>
        <div className="local-field-card">
          <label className="local-field-row">
            <span>Username</span>
            <input
              value={username}
              autoFocus
              disabled={Boolean(checkpoint.account)}
              onChange={(event) => setUsername(event.target.value)}
            />
          </label>
          <label className="local-field-row">
            <span>This Mac’s name</span>
            <input
              value={deviceName}
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
            <span>Invite (optional)</span>
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
        scope="Your keys are made on this Mac; only their public halves are registered"
        wide
        foot={
          <Foot back={() => go(checkpoint.managedLocal ? 'local' : 'checked')}>
            <Button
              variant="primary"
              disabled={
                busy ||
                (!checkpoint.account &&
                  (!username.trim() || !deviceName.trim()))
              }
              // Back from Protect lands here with the account already made.
              // The managed-local pane already continues in that case; this
              // one re-ran creation, which the agent refuses, stranding the
              // reader on a screen whose only button fails.
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
                  onChange={(event) => setUsername(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="This Mac’s name">
                <input
                  value={deviceName}
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
        {message ? <p className="crit">{message}</p> : null}
        <button className="lnk" onClick={() => openExisting('account')}>
          I already have an account on this server
        </button>
      </Pane>
    );
  else if (state === 'existing')
    content = (
      <Pane
        title="Your account"
        header={false}
        scope="This Mac needs a key of its own"
        wide
        foot={
          <Foot back={() => go(existingBack)}>
            <Button onClick={() => go('account')}>
              I don’t have an account yet
            </Button>
          </Foot>
        }
      >
        <h1>Add this Mac to your account</h1>
        <p className="lead">
          Your account already exists on {profile?.canonicalName}. Add this Mac
          with your backup phrase or approve it from another signed-in Mac.
        </p>
        <div className="two">
          <div className="pcard">
            <h3>Recover with your backup phrase</h3>
            <p>
              Enter all 17 tokens from your backup phrase to add this Mac as an
              owner device.
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
                  placeholder="token token token …"
                  value={recoveryPhrase}
                  onChange={(event) => setRecoveryPhrase(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="This Mac’s name">
                <input
                  value={deviceName}
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
          </div>
          <div className="pcard">
            <h3>
              {goCandidate
                ? 'Pair from the official FOKS CLI'
                : 'Pair from a Mac you already use'}
            </h3>
            {goCandidate ? (
              <>
                <p>
                  In Terminal, switch the official FOKS CLI to this account,
                  then run:
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
                  Confirm the account, paste its key-exchange code below, and
                  leave the command running until this Mac connects.
                </p>
              </>
            ) : (
              <p>
                On another signed-in Mac, open Settings › Recovery devices and
                choose Start pairing.
              </p>
            )}
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
                  onChange={(event) => setDeviceName(event.target.value)}
                />
              </InsetRow>
              <InsetRow label="Pairing phrase">
                <input
                  type="password"
                  aria-label="Pairing phrase"
                  placeholder="short phrase from the other Mac"
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
                Resume acceptance
              </Button>
            </div>
          </div>
          {goCandidate?.copyable ? (
            <div className="pcard">
              <h3>Copy this Mac’s CLI device</h3>
              <p>
                Advanced: both apps will use the same FOKS device. macOS may
                request Keychain access. Revoking it disables both, and CLI
                passphrase changes will not alter this desktop copy.
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
                disabled={busy || !recoveryTargetAlias}
                onClick={() => void copyGoCandidate()}
              >
                Copy existing device
              </Button>
            </div>
          ) : null}
        </div>
        {message ? <p className="crit">{message}</p> : null}
        <p className="hint">
          This Mac will become a device for{' '}
          <code>{checkpoint.account?.username ?? username}</code>. You can also
          use an enrolled YubiKey.
        </p>
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
        <h1>Keep access to your account</h1>
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
            Write down these 17 tokens and keep them somewhere other than this
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
                <p>Preparing the one-time phrase…</p>
              )}
              <label className="local-confirm">
                <input
                  type="checkbox"
                  checked={phraseWritten}
                  onChange={(event) => setPhraseWritten(event.target.checked)}
                />
                <span>I have written down all 17 tokens.</span>
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
          <div className="local-quiet-card">
            <b>Security key</b>
            <span>Enroll a YubiKey later from Settings.</span>
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
        <div className="local-success">
          <span className="local-success-mark" aria-hidden="true">
            ✓
          </span>
          <h1>Your Personal vault is ready</h1>
          <p className="lead">
            Your account is connected to the local server on this Mac.
          </p>
          <div className="local-vault-preview">
            <div className="local-vault-head">
              <Icon name="vault" />
              <b>Personal</b>
              <code>{profile?.canonicalName}</code>
            </div>
            <div className="local-vault-empty">
              {!accountStore
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
        scope="Set at least one now — the rest later, from Settings"
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
          Right now this Mac holds the only key to{' '}
          {checkpoint.account?.username}. Add at least one recovery method now.
          You can add more later under <b>Recovery devices</b> or{' '}
          <b>Security keys</b>.
        </p>
        <div className="two">
          <div className="pcard">
            <h3>Passphrase</h3>
            <p>
              Encrypts the keys this Mac keeps for you. It never leaves this Mac
              and the server never sees it.
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
            <p>Optional. Leave both empty to go without one.</p>
          </div>
          <div className="pcard">
            <h3>
              YubiKey <Chip>Later</Chip>
            </h3>
            <p>
              Enroll a hardware key later from <b>Settings › Security keys</b>.
            </p>
            <details className="dd">
              <summary>What to know first</summary>
              <div className="dbody">
                <ol>
                  <li>
                    <b>Preparing the card cannot be split or resumed.</b> If it
                    stops part way, recovery requires resetting its PIV applet,
                    which erases everything on it.
                  </li>
                  <li>
                    <b>It must still hold its factory management key.</b> A card
                    that has already been managed will refuse.
                  </li>
                  <li>
                    <b>The unlock code is yours to choose and keep.</b> Nothing
                    generates one for you or shows it later.
                  </li>
                </ol>
              </div>
            </details>
            <Button
              onClick={() =>
                setMessage(
                  'A YubiKey is enrolled later from Settings › Security keys.',
                )
              }
            >
              Enroll a YubiKey…
            </Button>
          </div>
          <div className="pcard">
            <h3>
              Backup phrase{' '}
              <Chip tone={checkpoint.backupCommitted ? undefined : 'warn'}>
                {checkpoint.backupCommitted ? 'Written down' : 'Not yet'}
              </Chip>
            </h3>
            <p>
              Write down these 17 tokens to recover your account if every device
              is lost. FOKS shows them once.
            </p>
            <Button
              disabled={checkpoint.backupCommitted || busy}
              onClick={() => go('phrase')}
            >
              {checkpoint.backupCommitted
                ? 'Shown once — done'
                : 'Show my phrase'}
            </Button>
          </div>
        </div>
        {message ? <p className="crit">{message}</p> : null}
        {state === 'phrase' ? (
          <SheetDialog
            width="wide"
            onClose={() => go('protect')}
            glyph={<Icon name="key" />}
            title="Write these tokens down"
            subtitle="Shown once and cannot be copied"
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
                Anyone with these tokens can access your account. Store them
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
                <p>Preparing the one-time phrase…</p>
              )}
              <button
                type="button"
                aria-label="I have written these 17 tokens down"
                className={`check${phraseWritten ? ' on' : ''}`}
                onClick={() => setPhraseWritten((value) => !value)}
              >
                <span className="bx">{phraseWritten ? '✓' : ''}</span>I have
                written these 17 tokens down
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
              I’ll come back later
            </Button>
          </Foot>
        }
      >
        <h1>
          Waiting for {admin} to add {checkpoint.account?.username}
        </h1>
        <p className="lead">
          Your account {checkpoint.account?.username} on{' '}
          {profile?.canonicalName} exists. {adminShort} must add you to{' '}
          <b>{group}</b>. Check again after they do.
        </p>
        <div className="two">
          <div className="col">
            <div className="pcard">
              <h3>Send {adminShort} this</h3>
              <CopyBox
                text={`Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`}
                onCopy={(value) =>
                  void bridge
                    .copyText(value)
                    .then(() => toasts.show('Sentence copied.'))
                }
              >
                “Add {checkpoint.account?.username} on {profile?.canonicalName}{' '}
                to {group}”
              </CopyBox>
              <p>
                {adminShort} needs your username exactly as written. Nothing
                else — no code, no link.
              </p>
            </div>
            <Inset className="checklist">
              <InsetRow label={<Icon name="people" />}>
                <b>{group} will appear under GROUPS</b>
                <span className="hint">
                  FOKS lists groups after it checks the server.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="eye" />}>
                <b>You’ll see what your role lets you read</b>
                <span className="hint">
                  Roles are <b>Member</b>, <b>Admin</b>, and <b>Owner</b>.
                  Members also have a visibility level. Items above your role or
                  visibility level remain locked.
                </span>
              </InsetRow>
              <InsetRow label={<Icon name="door" />}>
                <b>Quitting is fine</b>
                <span className="hint">
                  Your account and pinned server remain on this Mac. Reopen FOKS
                  and continue from Get started.
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
                  <Chip>
                    {message ? 'Checked just now' : 'Not checked yet'}
                  </Chip>
                  {message ? <>Not yet — only Personal is listed.</> : null}
                </span>
              </div>
              <p>
                FOKS checks for the group when it opens and when you select{' '}
                <b>Check now</b>. Checks stop when FOKS is closed.
              </p>
              <Band label="Group discovery">
                <b>Check now</b> searches for groups using your authenticated
                account. FOKS also checks when it opens.
              </Band>
              {message ? <div className="res">{message}</div> : null}
              <p className="note">
                You can use <b>Personal</b> while you wait. If {adminShort} has
                already added you, confirm that they entered{' '}
                <code>{checkpoint.account?.username}</code>.
              </p>
              <details className="dd">
                <summary>Details</summary>
                <p>
                  Re-reads {checkpoint.account?.username}’s own signed chain on{' '}
                  {profile?.canonicalName}, then lists groups. A group appears
                  only when that authenticated answer and the refreshed catalog
                  agree.
                </p>
              </details>
            </div>
            <div className="pcard">
              <h3>Use your Personal vault</h3>
              <p>
                {PERSONAL_FIXED} Put your own logins in <b>Personal</b> now;
                nothing in it is visible to {group}.
              </p>
              <Button onClick={() => go('checklist-invited')}>
                Open Personal
              </Button>
            </div>
          </div>
        </div>
      </Pane>
    );
  else if (state === 'create-group')
    content = (
      <Pane
        title="Create a group"
        scope="Or skip — Groups is always in the sidebar"
        header={false}
        foot={
          <Foot back={() => go('protect')}>
            <button
              className="lnk"
              onClick={() => send({ type: 'skip-group' })}
            >
              Skip this step
            </button>
            <Button
              variant="primary"
              disabled={busy || !groupName.trim()}
              onClick={() => void createGroup()}
            >
              Create group
            </Button>
          </Foot>
        }
      >
        <h1>Create a group</h1>
        <p className="lead">
          {PERSONAL_FIXED} Start one now, or skip and do it later from{' '}
          <b>Groups</b>.
        </p>
        <SectionLabel>Your first group</SectionLabel>
        <Inset>
          <InsetRow label="Group name">
            <input
              value={groupName}
              onChange={(event) => setGroupName(event.target.value)}
            />
          </InsetRow>
          <GroupKindChoice
            value={groupKind}
            onChange={setGroupKind}
            server={profile?.canonicalName ?? address}
          />
        </Inset>
        {message ? <p className="crit">{message}</p> : null}
        <p className="hint">
          You can add people by username after creating a group.
        </p>
      </Pane>
    );
  else if (state === 'checklist-invited' || state === 'checklist-own')
    content = (
      <Pane
        title="Get started"
        subtitle={`${completedFirstRunSteps(checkpoint)} of 5 done`}
        wide
      >
        <p className="lead">
          {checkpoint.path === 'invited'
            ? 'You closed FOKS while waiting.'
            : 'You stopped part way last time.'}{' '}
          Completed steps are saved. Continue with the remaining steps below.
        </p>
        <Inset className="checklist">
          <InsetRow label="✓">
            <b>Prepare this Mac</b>
            <span className="hint">
              Agent ready. Encrypted client state initialised.
            </span>
          </InsetRow>
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
              {profile?.canonicalName} · this Mac is{' '}
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
                ? `Skipped. This Mac holds the only key to ${checkpoint.account?.username}; a passphrase, YubiKey or 17-token backup phrase gives you a second way in.`
                : 'Passphrase set · backup phrase written down · YubiKey later, from Settings'}
            </span>
          </InsetRow>
          <InsetRow
            label={
              checkpoint.path === 'invited' && !checkpoint.added
                ? '5'
                : checkpoint.groupSkipped
                  ? '!'
                  : '✓'
            }
            action={
              <span className="checklist-actions">
                {checkpoint.path === 'invited' ? (
                  <Button
                    size="sm"
                    icon="copy"
                    onClick={() =>
                      void bridge
                        .copyText(
                          `Add ${checkpoint.account?.username} on ${profile?.canonicalName} to ${group}`,
                        )
                        .then(() => toasts.show('Sentence copied.'))
                    }
                  >
                    Copy the sentence
                  </Button>
                ) : null}
                <Button
                  size="sm"
                  onClick={() =>
                    go(
                      checkpoint.path === 'invited'
                        ? 'waiting'
                        : 'create-group',
                    )
                  }
                >
                  {checkpoint.path === 'invited'
                    ? 'Check now'
                    : 'Create a group…'}
                </Button>
              </span>
            }
          >
            <b>
              {checkpoint.path === 'invited'
                ? `Waiting for ${admin} to add you`
                : 'Create a group'}
            </b>
            <span className="hint">
              {checkpoint.path === 'invited'
                ? `${group} isn’t listed yet. Check again after ${adminShort} adds you, or send them the sentence above.`
                : `${PERSONAL_FIXED} You become its Owner and add people by username from the group’s settings.`}
            </span>
            {checkpoint.path === 'invited' ? (
              <Band label="Group discovery">
                Check now searches for groups using your authenticated account.
              </Band>
            ) : null}
          </InsetRow>
        </Inset>
        {!(checkpoint.passphraseSet || checkpoint.backupCommitted) ? (
          <div className="checklist-notice">
            <Notice
              eyebrow="Save recovery phrase · skipped"
              title={`This Mac holds the only key to ${checkpoint.account?.username}`}
            >
              Lose it and the account is gone — a passphrase, YubiKey or
              17-token backup phrase is a second way in. Nothing else in the
              list is blocked by this.
            </Notice>
          </div>
        ) : null}
      </Pane>
    );
  else
    content = (
      <Pane
        title={
          state === 'added' ? group : (checkpoint.group?.name ?? groupName)
        }
        subtitle={`${checkpoint.group?.kind === 'adhoc' ? 'ad-hoc' : 'named'} group on ${profile?.canonicalName}`}
        wide
      >
        <Notice
          eyebrow={`${state === 'added' ? group : checkpoint.group?.name} · ${profile?.canonicalName}`}
          title={
            state === 'added'
              ? `You’re in ${group}`
              : `${checkpoint.group?.name ?? groupName} exists`
          }
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
                Open your vault
              </Button>
            </>
          }
        >
          {state === 'added' ? (
            <>
              <p>
                {facts?.admin ? (
                  <>
                    {admin} added <code>{checkpoint.account?.username}</code> as
                    a Member, and {group} is listed here now: its{' '}
                    {
                      world.items.filter((item) => item.store === addedStore)
                        .length
                    }{' '}
                    items are listed, and what your role can read is open.
                  </>
                ) : (
                  <>
                    Authenticated discovery now lists{' '}
                    <code>{checkpoint.account?.username}</code> in this group.
                    The items below are exactly what this Mac can read now.
                  </>
                )}
              </p>
              <p className="fn">
                Groups appear after this Mac creates them or checks for groups
                it has joined.
              </p>
            </>
          ) : (
            <>
              <p>
                You are its Owner and nobody else is in it yet. Add people by
                username from the group’s settings — they need an account on{' '}
                {profile?.canonicalName} first: send them the address, ask for
                the username they picked, then add them as Member, Admin or
                Owner.
              </p>
              <p className="fn">
                Select Resume to continue interrupted group creation without
                repeating completed steps. Pairing is under{' '}
                <b>Settings › Recovery devices</b>, alongside your 17-token
                backup phrase.
              </p>
            </>
          )}
        </Notice>
        {state === 'done' ? (
          <div className="empty">
            <Icon name="key" />
            <h2>No items here</h2>
            <p>
              Anything saved in {checkpoint.group?.name ?? groupName} is read by
              everyone in it at their role. Nothing is listed until something is
              written.
            </p>
            <Button onClick={() => onNavigate({ kind: 'all' })}>New</Button>
          </div>
        ) : (
          <>
            <div className="hdr">
              <span />
              <span>Name</span>
              <span>Readable by</span>
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
          </>
        )}
      </Pane>
    );

  const appMode = [
    'added',
    'done',
    'checklist-invited',
    'checklist-own',
  ].includes(state);
  const canCancelSetup = world.stores.some((store) => store.kind === 'account');
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
            checkpoint.managedLocal && !checkpoint.account
              ? () => send({ type: 'choose', path: 'own' })
              : undefined
          }
          onRecoverAccount={
            checkpoint.managedLocal && !checkpoint.account
              ? () => selectManagedProfile(true)
              : undefined
          }
          recoverEnabled={Boolean(managedReport)}
          onCancel={
            canCancelSetup ? () => onNavigate({ kind: 'all' }) : undefined
          }
        />
      )}
      <main className="main first-run-main">{content}</main>
      {state === 'added' ? (
        <AddedDetails world={world} storeId={addedStore} />
      ) : null}
    </>
  );
}
