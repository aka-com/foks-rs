import type { ReactNode } from 'react';
import {
  Button,
  Chip,
  Icon,
  KindIcon,
  SectionLabel,
  Inset,
  InsetRow,
} from '../components';
import type { AgentSnapshot, RoleWire } from '../model';
import { displayPath, fmtSize, kindOf, parseRole, formatRole } from '../model';
import type { FilterKind } from '../components';
import type { Location } from '../location';
import { NavRow, Sidebar, TrafficStrip } from '../shell/sidebar';
import type { RailAgentState } from '../shell/sidebar';
import { PageHeader } from '../shell/page-header';
import {
  completedFirstRunSteps,
  firstRunNextStep,
  firstRunNextStepState,
  firstRunStepCount,
  type FirstRunCheckpoint,
  type FirstRunPath,
  type FirstRunStateName,
} from '../first-run-state';
export const stepOf = (state: FirstRunStateName): number => {
  if (state === 'boot') return 0;
  if (state === 'local') return 0;
  if (state === 'who') return 0;
  if (['address', 'no-address', 'checked', 'compare', 'error'].includes(state))
    return 1;
  if (
    state === 'account' ||
    state === 'existing' ||
    state === 'identity-pending' ||
    state === 'operation-pending'
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
    state === 'identity-pending' ||
    state === 'operation-pending'
  )
    return 1;
  if (state === 'protect' || state === 'phrase') return 2;
  return 3;
};

export function SetupSidebar({
  checkpoint,
  pendingPath,
  native = false,
  onCancel,
  cancelDisabled = false,
  onRestart,
  restartDisabled = false,
  restartReason,
  onAnotherServer,
  onRecoverAccount,
  recoverEnabled = false,
  blocked = false,
}: {
  checkpoint: FirstRunCheckpoint;
  /** The OS draws the window controls over the step list's own drag strip. */
  native?: boolean;
  /** Path selected on the 'who' screen before confirmation. */
  pendingPath?: FirstRunPath | null;
  onCancel?: () => void;
  cancelDisabled?: boolean;
  onRestart?: () => void;
  restartDisabled?: boolean;
  restartReason?: string;
  onAnotherServer?: () => void;
  onRecoverAccount?: () => void;
  recoverEnabled?: boolean;
  /** Connection status displayed in the rail footer. */
  agent?: RailAgentState;
  blocked?: boolean;
}): ReactNode {
  // Restart returns to `who`. While already there it would only reopen the
  // same first step, so omit the action until setup has moved beyond it.
  const restart = onRestart && checkpoint.state !== 'who' ? (
    <button
      type="button"
      className="nav"
      disabled={blocked || restartDisabled}
      title={restartReason}
      onClick={onRestart}
    >
      <Icon name="flag" />
      <span className="t">Restart setup</span>
    </button>
  ) : null;
  if (checkpoint.managedLocal) {
    const current = localStepOf(checkpoint.state);
    const labels = ['Local server', 'Create account', 'Account recovery'];
    return (
      <nav className="side rail setup-side" aria-label="Setup steps">
        <TrafficStrip native={native} />
        <div className={`setup-steps${blocked ? ' is-blocked' : ''}`}>
          {labels.map((label, index) => (
            <div
              key={label}
              className={`nav setup-step${index === current ? ' on' : ''}${index < current ? ' done' : ''}`}
              aria-current={index === current ? 'step' : undefined}
            >
              <span className="setup-mark" aria-hidden="true">
                {index < current ? <Icon name="check" /> : index + 1}
              </span>
              <span className="t">{label}</span>
            </div>
          ))}
        </div>
        <div className="side-bottom">
          {onAnotherServer ? (
            <button
              type="button"
              className="nav"
              disabled={blocked}
              onClick={onAnotherServer}
            >
              <Icon name="server" />
              <span className="t">Connect to another server</span>
            </button>
          ) : null}
          {onRecoverAccount ? (
            <button
              type="button"
              className="nav"
              disabled={blocked || !recoverEnabled}
              onClick={onRecoverAccount}
            >
              <Icon name="user" />
              <span className="t">Recover account</span>
            </button>
          ) : null}
          {restart}
          {onCancel ? (
            <button
              type="button"
              className="nav"
              disabled={blocked || cancelDisabled}
              onClick={onCancel}
            >
              <Icon name="close" />
              <span className="t">
                {checkpoint.account ||
                checkpoint.provisionedAccount ||
                checkpoint.provisioning
                  ? 'Finish later'
                  : 'Leave setup'}
              </span>
            </button>
          ) : null}
        </div>
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
    'Recovery phrase',
    ...(path === 'invited' ? ['Join a team'] : []),
    'Complete',
  ];
  return (
    <nav className="side rail setup-side" aria-label="Setup steps">
      <TrafficStrip native={native} />
      <div className={`setup-steps${blocked ? ' is-blocked' : ''}`}>
        {labels.map((label, index) => (
          <div
            key={label}
            className={`nav setup-step${index === current ? ' on' : ''}${index < current ? ' done' : ''}`}
            aria-current={index === current ? 'step' : undefined}
          >
            <span className="setup-mark">
              {index < current ? <Icon name="check" /> : index + 1}
            </span>
            <span className="t">{label}</span>
          </div>
        ))}
      </div>
      <div className="side-bottom">
        {onCancel ? (
          <>
            {restart}
            <button
              type="button"
              className="nav"
              disabled={blocked || cancelDisabled}
              onClick={onCancel}
            >
              <Icon name="close" />
              <span className="t">
                {checkpoint.account ||
                checkpoint.provisionedAccount ||
                checkpoint.provisioning
                  ? 'Finish later'
                  : 'Leave setup'}
              </span>
            </button>
          </>
        ) : null}
      </div>
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
  // Completed setup leaves no checklist row in the rail.
  if (completed >= total) return null;
  // One row, one name: the row reads the same before and after an account
  // exists.
  const name = 'Continue setup';
  return (
    <>
      <NavRow
        active={active}
        glyph={<Icon name="flag" />}
        name={name}
        title={name}
        onSelect={() =>
          onNavigate({
            kind: 'first-run',
            step: firstRunNextStepState(checkpoint),
            path: checkpoint.path,
          })
        }
      />
    </>
  );
}

export function FirstRunAppSidebar({
  snapshot,
  checkpoint,
  groupName,
  location,
  native,
  onNavigate,
  onReenter,
  collapsed = false,
  onToggleCollapsed,
  agent = 'ready',
  blocked = false,
  devicesAlert = null,
}: {
  snapshot: AgentSnapshot;
  native: boolean;
  checkpoint: FirstRunCheckpoint;
  groupName: string;
  location: Location;
  onNavigate: (location: Location) => void;
  onReenter: () => void;
  /** The rail's own width, carried across first run as on every other screen. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  agent?: RailAgentState;
  blocked?: boolean;
  /** The Devices tab's dot, as the shell computes it elsewhere. */
  devicesAlert?: { description: string } | null;
}): ReactNode {
  // Setup progress is the rail's card; the invitation note keeps the status
  // slot under it.
  const completed = completedFirstRunSteps(checkpoint);
  const total = firstRunStepCount(checkpoint);
  const setup =
    completed < total
      ? {
          done: completed,
          total,
          next: firstRunNextStep(checkpoint),
          onContinue: () =>
            onNavigate({
              kind: 'first-run',
              step: firstRunNextStepState(checkpoint),
              path: checkpoint.path,
            }),
        }
      : undefined;
  const status = (
    <>
      {checkpoint.path === 'invited' && !checkpoint.added ? (
        <p className="side-note">
          {groupName} will appear under Teams once your access is approved. FOKS
          checks for team access at launch. You can also click Check now.
        </p>
      ) : null}
    </>
  );
  return (
    <Sidebar
      snapshot={snapshot}
      location={location}
      attention={checkpoint.path === 'invited' && !checkpoint.added ? 1 : 0}
      // A skipped recovery step is the same fact the Devices dot reports
      // everywhere else — an account with no backup — and the registry the
      // shell reads has not been filled during setup, so the checkpoint
      // answers for it here.
      devicesAlert={
        devicesAlert ??
        (checkpoint.protectSkipped
          ? { description: 'An account has no paper key' }
          : null)
      }
      nativeChrome={native}
      onNavigate={onNavigate}
      setup={setup}
      status={status}
      onReenter={onReenter}
      collapsed={collapsed}
      onToggleCollapsed={onToggleCollapsed}
      agent={agent}
      blocked={blocked}
    />
  );
}

export function AddedDetails({
  snapshot,
  storeId,
}: {
  snapshot: AgentSnapshot;
  storeId?: string;
}): ReactNode {
  const store = snapshot.stores.find((candidate) => candidate.id === storeId);
  const item = snapshot.items.find(
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
            {store?.name ?? 'this team'}
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
          Access to this item depends on your role in the team.
        </p>
        <SectionLabel>Info</SectionLabel>
        <div className="meta">
          <b>Location</b>
          <code>{displayPath(item.path)}</code>
          <b>Kind</b>
          <span>{kind}</span>
          <b>Version</b>
          <span>{item.version}</span>
          <b>Size</b>
          <span>
            {fmtSize(item.size)}
          </span>
          <b>Read permission</b>
          <Chip>{roleText(item.read)}</Chip>
          <b>Write permission</b>
          <Chip>{roleText(item.write)}</Chip>
        </div>
        <SectionLabel>Sharing</SectionLabel>
        <p>
          Members of {store?.name ?? 'this team'} with the required role or
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

export function Foot({
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

export function Pane({
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
