import { useGroupOperationController } from './groups/operation-controller';
import {
  attemptMutation,
  reportMutationOutcome,
} from '../commands/command-policy';
import { synchronizeApplied } from '../operation-outcome';
import {
  requireWorkflow,
  workflowAvailability,
  workflowMessage,
} from '../model/workflow-availability';
import { useTabSheetState } from '../navigation-guard';
import { InvitationRecovery } from '../components/invitation-recovery';
import { InvitationPanel } from '../components/invitation-panel';
import { useTeamRequestCounts } from '../operation-queries';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  Field,
  Icon,
  Inset,
  InsetRow,
  MenuButton,
  MenuItem,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SegmentedControl,
  SheetDialog,
  tabId,
  tabPanelId,
  Tabs,
  Toggle,
} from '../components';
import {
  actionableGroupMember,
  catalog,
  storeOperationAvailability,
  groupDetailFailure,
  hue,
  isMachine,
  parseRole,
  partiesOf,
  partyName,
  plural,
  readersOf,
  roleChipLabel,
  roleRank,
  serverDisplayName,
  serverName as displayServerName,
  serverOf,
  shortId,
  storeDescriptionState,
  storeOf,
  visibilityOf,
} from '../model';
import type {
  AccountStore,
  AvailabilityOptions,
  FederationEntry,
  GroupDetailFailure,
  Item,
  Party,
  Store,
  StoreRef,
  TeamStore,
  AgentSnapshot,
} from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type { RoleDto } from '../bridge';
import { useSidebarInbox } from '../chat/inbox-provider';
import { listChannels } from '../chat/presentation';
import { GROUP_SETTINGS_TABS } from '../location';
import type { GroupSettingsTab, Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { PageHeader } from '../shell/page-header';
import { NewChatSheet } from './chat-new';
import {
  ChannelsTab,
  FilesTab,
  IncompleteGroupPage,
  itemCountOf,
} from './group-tabs';
import { GroupMark } from './group-mark';
import { manageReason } from './group-model';
import { StoreAccessTakeover } from './store-access';
import { useToast } from '/kit/toasts';

type Tab = GroupSettingsTab;
export type GroupSheetKind = 'add' | 'demote' | 'remove' | 'admit' | 'create';
type Sheet = GroupSheetKind | null;

const VIS_MIN = -32768;
const VIS_MAX = 32767;
/**
 * What the two typed fields of the group sheets are drawn with. A field still
 * reading its suggestion has not been typed into, so nothing is asked about
 * it when the reader leaves.
 */
const SUGGESTED_MEMBER = 'jules.park';
const SUGGESTED_GROUP = 'Platform';

/**
 * The name a group is created under, or null when FOKS would refuse it.
 * Mirrors `server_team_name` in foks-client-app: runs of whitespace become one
 * underscore, and the result must normalize (lowercased, dots and dashes as
 * underscores) to 3 to 25 letters, digits and single underscores.
 */
export function serverTeamName(name: string): string | null {
  const folded = name.trim().split(/\s+/).filter(Boolean).join('_');
  const normalized = folded.toLowerCase().replace(/[.-]/g, '_');
  return /^[a-z0-9]+(?:_[a-z0-9]+)*$/.test(normalized) &&
    normalized.length >= 3 &&
    normalized.length <= 25
    ? folded
    : null;
}
/** The base the group page's tab and panel ids are derived from. */
const GROUP_TABS = 'group-sections';
/** Why the join policy cannot be changed. */
const stateName = (): string =>
  typeof window === 'undefined'
    ? ''
    : (new URLSearchParams(window.location.search).get('state') ?? '');

function itemsOf(snapshot: AgentSnapshot, store: string): Item[] {
  return catalog(snapshot).filter((item) => item.store === store);
}

function readsOf(snapshot: AgentSnapshot, party: Party): Item[] {
  return itemsOf(snapshot, party.store).filter((item) =>
    readersOf(snapshot, item)?.some(
      (candidate) => candidate.party_id_hex === party.party_id_hex,
    ),
  );
}

function canTarget(snapshot: AgentSnapshot, party: Party): boolean {
  if (!actionableGroupMember(snapshot, party)) return false;
  const parties = partiesOf(snapshot, party.store);
  const mine = parties.find((candidate) => candidate.label === 'you');
  if (!mine) return false;
  const myRank = roleRank(mine.destination_role);
  const targetRank = roleRank(party.destination_role);
  if (myRank < 2) return false;
  if (targetRank >= 3 && myRank < 3) return false;
  return true;
}

function demotionFor(party: Party): RoleDto | null {
  const role = parseRole(party.destination_role);
  if (!role) return null;
  if (role.kind === 'owner' || role.kind === 'admin')
    return { role: 'Member', visibility: 0 };
  const visibility = role.visibility ?? 0;
  return visibility > VIS_MIN
    ? { role: 'Member', visibility: visibility - 1 }
    : null;
}

function roleText(party: Party): string {
  return fmtRole(party.destination_role);
}

/** One notation for a role everywhere on this page: "Owner", "Member · sees level 0". */
function fmtRole(role: Item['read']): string {
  const parsed = parseRole(role);
  return parsed
    ? roleChipLabel(parsed)
    : typeof role === 'string'
      ? role
      : role.role;
}

/**
 * A role as a row reads it: one chip, with the visibility band inside it when
 * the role is a Member, and no second line.
 */
function RoleChip({
  role,
}: {
  role: Item['read'] | RoleDto | null | undefined;
}): ReactNode {
  const parsed = role ? parseRole(role) : null;
  return (
    <span className="rolecell">
      <Chip>
        {parsed
          ? roleChipLabel(parsed)
          : role
            ? typeof role === 'string'
              ? role
              : role.role
            : '—'}
      </Chip>
    </span>
  );
}

function sortRoster(parties: readonly Party[]): Party[] {
  return [...parties].sort((left, right) => {
    const rank =
      roleRank(right.destination_role) - roleRank(left.destination_role);
    if (rank) return rank;
    if (left.label === 'you' && right.label !== 'you') return -1;
    if (right.label === 'you' && left.label !== 'you') return 1;
    return 0;
  });
}

function tabFromState(name: string): Tab {
  if (name === 'settings' || name === 'danger' || name === 'rekey-menu')
    return 'settings';
  if (name === 'group-channels') return 'channels';
  if (name === 'group-files') return 'files';
  return 'people';
}

function useCopyText(
  bridge: Bridge,
  onError: (error: unknown) => void,
): (text: string, message: string) => Promise<void> {
  const toasts = useToast();
  return async (text, message) => {
    try {
      await bridge.copyText(text);
      toasts.show(message);
    } catch (error) {
      onError(error);
    }
  };
}

/**
 * What a member row's second line says: the kind of party, and nothing else.
 * Only people and machines are drawn with `PartyRow`; a party that stands for
 * another team is drawn with `FederationEntryRow` or `TeamPartyRow` instead,
 * in the same Members list, each with its own "team on …" caption.
 */
function partySubtitle(party: Party): string {
  return isMachine(party) ? 'machine' : 'person';
}

/** Why a member cannot be changed from here, for a disabled menu item. */
function targetReason(
  snapshot: AgentSnapshot,
  store: Store,
  party: Party,
  manageable: boolean,
): string {
  if (store.kind === 'team' && store.team_kind === 'adhoc')
    return 'Memberships can’t be changed in an ad-hoc team.';
  if (!manageable)
    return 'Only an Admin or an Owner can change this team’s members.';
  if (party.label === 'you')
    return 'You cannot change your own role or remove your own account.';
  if (party.party_kind !== 'user' || !party.locally_manageable)
    return 'Members of an admitted team are managed on their own server and cannot be changed or removed one by one.';
  const mine = partiesOf(snapshot, party.store).find(
    (candidate) => candidate.label === 'you',
  );
  if (
    mine &&
    roleRank(party.destination_role) >= roleRank(mine.destination_role)
  )
    return 'An Admin cannot change another Admin or the Owner.';
  return 'This member cannot be changed from this device.';
}

/** The mark before a member's name: an initial, or a machine's glyph. */
function PartyMark({ party }: { party: Party }): ReactNode {
  const machine = isMachine(party);
  const name = partyName(party);
  return (
    <span
      className="kico round"
      style={{ background: machine ? 'var(--c-none)' : hue(name) }}
    >
      {machine ? <Icon name="term" /> : name.slice(0, 1).toUpperCase()}
    </span>
  );
}

/**
 * One member: who they are, what kind of party, the role they hold, and a menu
 * of the actions that apply to them.
 */
function PartyRow({
  snapshot,
  store,
  party,
  manageable,
  menuOpen,
  onSheet,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  party: Party;
  manageable: boolean;
  menuOpen: boolean;
  onSheet: (sheet: Sheet, party?: Party) => void;
}): ReactNode {
  const actionable = manageable && canTarget(snapshot, party);
  const reason = actionable
    ? undefined
    : targetReason(snapshot, store, party, manageable);
  const name = partyName(party);
  const lowerable = Boolean(demotionFor(party));
  return (
    // Only people and machines reach this row, and their admission is their
    // membership: there is no inactive state to dim.
    <div className="prow">
      <span className="who2">
        <PartyMark party={party} />
        <span className="t">
          <b>
            <span>{name}</span>
            {party.label ? <Chip tone="you">you</Chip> : null}
          </b>
          <small>{partySubtitle(party)}</small>
        </span>
      </span>
      <span className="rowtail">
        <RoleChip role={party.destination_role} />
        <MenuButton
          variant="quiet"
          icon="more"
          trailingIcon={null}
          label=""
          menuLabel={`Actions for ${name}`}
          aria-label={`Actions for ${name}`}
          defaultOpen={menuOpen}
        >
          {(close) => (
            <>
              <MenuItem
                reason={
                  actionable
                    ? lowerable
                      ? undefined
                      : 'This member is already at the lowest role.'
                    : reason
                }
                title="Roles can only be lowered. To raise one, remove the member and add them again, which rotates the team key."
                onClick={() => {
                  close();
                  onSheet('demote', party);
                }}
              >
                Lower role…
              </MenuItem>
              <MenuItem
                danger
                reason={actionable ? undefined : reason}
                title="Removes this member and rotates the team key."
                onClick={() => {
                  close();
                  onSheet('remove', party);
                }}
              >
                Remove…
              </MenuItem>
            </>
          )}
        </MenuButton>
      </span>
    </div>
  );
}

/**
 * The one standing condition a tab states before its own content. A group
 * whose setup never finished never reaches a tab at all, so the only band left
 * here is the ad-hoc share's fixed membership.
 */
function SituationBand({ store, tab }: { store: Store; tab: Tab }): ReactNode {
  if (
    store.kind === 'team' &&
    store.team_kind === 'adhoc' &&
    tab !== 'settings'
  ) {
    return (
      <Band severity="info" label="Ad-hoc team">
        Memberships can’t be changed.
      </Band>
    );
  }
  return null;
}

/** A sub-section of the Members tab: its label, its count, and its rows. */
function MemberSection({
  title,
  count,
  children,
}: {
  title: string;
  count?: string;
  children: ReactNode;
}): ReactNode {
  return (
    <>
      <SectionLabel>
        {title}
        {count ? <span className="count">— {count}</span> : null}
      </SectionLabel>
      <div className="rt bare">{children}</div>
    </>
  );
}

/** The alias a roster party that stands for another group is listed under. */
function teamPartyName(party: Party): string {
  return party.team_name?.split(' @ ')[0] ?? partyName(party);
}

/** The server a roster party's group is held on, as its `team_name` says. */
function teamPartyHost(party: Party): string {
  const [, host] = party.team_name?.split(' @ ') ?? [];
  return host ?? 'another server';
}

/**
 * The groups admitted here, each one drawn once.
 *
 * An admission is normally an entry and the roster party it matches, listed as
 * the entry. A roster party that matches no entry is a missing record, and one
 * that matches several is an ambiguous record: both are still members of this
 * group, so they are listed as the party the roster holds, and the entries an
 * ambiguous party matches are left to that one row rather than repeated.
 */
interface AdmittedGroups {
  entries: FederationEntry[];
  unmatched: Party[];
  ambiguous: Party[];
}

function admittedGroups(snapshot: AgentSnapshot, store: Store): AdmittedGroups {
  const entries = snapshot.federation.filter(
    (entry) => entry.store === store.id,
  );
  const unmatched: Party[] = [];
  const ambiguous: Party[] = [];
  const claimed = new Set<FederationEntry>();
  for (const party of partiesOf(snapshot, store.id)) {
    if (party.party_kind === 'user') continue;
    const matches = entries.filter(
      (entry) =>
        entry.remote_team_id_hex === party.party_id_hex &&
        (!party.scoped_host_id_hex ||
          entry.remote_host_id_hex === party.scoped_host_id_hex),
    );
    if (!matches.length) unmatched.push(party);
    else if (matches.length > 1) {
      ambiguous.push(party);
      for (const entry of matches) claimed.add(entry);
    }
  }
  return {
    entries: entries.filter((entry) => !claimed.has(entry)),
    unmatched,
    ambiguous,
  };
}

/**
 * How many members the group has: its people and machines, plus the admitted
 * groups counted once each, however their records read.
 */
function memberCountOf(snapshot: AgentSnapshot, store: Store): number {
  const { entries, unmatched, ambiguous } = admittedGroups(snapshot, store);
  return (
    partiesOf(snapshot, store.id).filter((party) => party.party_kind === 'user')
      .length +
    entries.length +
    unmatched.length +
    ambiguous.length
  );
}

/**
 * What an admitted team's row keeps out of its caption: the server, the host
 * this device knows it by, and the operation that admitted it. One block of
 * text, so a failure can be reported without reading it off the screen.
 */
function admissionDetails(entry: FederationEntry, remoteName: string): string {
  return [
    `team ${entry.remote_team_alias} on ${remoteName}`,
    `host ${entry.remote_host_id_hex}`,
    `team id ${entry.remote_team_id_hex}`,
    ...(entry.operation_id_hex ? [`operation ${entry.operation_id_hex}`] : []),
  ].join('\n');
}

/**
 * A roster party that stands for another group whose admission record cannot be
 * resolved: the group as the roster names it, its role, and what is wrong.
 */
function TeamPartyRow({
  party,
  chip,
  chipTitle,
}: {
  party: Party;
  chip: string;
  chipTitle: string;
}): ReactNode {
  const name = teamPartyName(party);
  return (
    <div className="prow">
      <span className="who2">
        <span
          className="kico round"
          style={{ background: hue(name) }}
          aria-hidden="true"
        >
          {name.slice(0, 1).toUpperCase()}
        </span>
        <span className="t">
          <b>
            <span>{name}</span>
          </b>
          <small>team on {teamPartyHost(party)}</small>
        </span>
      </span>
      <span className="rowtail">
        <RoleChip role={party.destination_role} />
        <Chip tone="warn" title={chipTitle}>
          {chip}
        </Chip>
      </span>
    </div>
  );
}

/**
 * One team admitted from another server, drawn as a member row: it keeps its
 * own facts — the admission state, the host id and the operation id, because
 * restoring access needs the operation id — but its caption says what it is,
 * so it is never mistaken for a person.
 */
function FederationEntryRow({
  snapshot,
  entry,
  onRerun,
  onRemove,
  onCopy,
  manageable,
}: {
  snapshot: AgentSnapshot;
  entry: FederationEntry;
  onRerun: (operationId: string) => void;
  onRemove: (entry: FederationEntry) => void;
  /** Copies a row's identifiers, with the toast the page uses elsewhere. */
  onCopy: (text: string, message: string) => void;
  manageable: boolean;
}): ReactNode {
  const remoteServer = snapshot.servers.find(
    (server) => server.id === entry.remote_profile,
  );
  const remoteName = remoteServer
    ? serverDisplayName(remoteServer)
    : entry.remote_profile;
  const memberReason =
    'Every member of an admitted team holds the role shown on its row. They are managed on their own server and cannot be changed or removed one by one.';
  return (
    <div
      className="prow"
      key={`${entry.remote_host_id_hex}|${entry.remote_team_id_hex}`}
    >
      <span className="who2">
        <span
          className="kico round"
          style={{ background: hue(entry.remote_team_alias) }}
          aria-hidden="true"
        >
          {entry.remote_team_alias.slice(0, 1).toUpperCase()}
        </span>
        <span className="t">
          <b>
            <span>{entry.remote_team_alias}</span>
          </b>
          {/* The server the team lives on is what a reader of the roster
              needs. The host and operation identifiers are for reporting a
              failed admission, so they are in the row's menu instead. */}
          <small>team on {remoteName}</small>
        </span>
      </span>
      <span className="rowtail">
        <RoleChip role={entry.destination} />
        <Chip tone={entry.active ? 'ok' : 'warn'}>
          {entry.active ? 'Active' : 'Inactive'}
        </Chip>
        {!entry.active && entry.operation_id_hex ? (
          <Button
            size="sm"
            icon="again"
            disabled={!manageable}
            title={
              manageable
                ? undefined
                : 'Only an Admin or an Owner can restore this admission.'
            }
            onClick={() => onRerun(entry.operation_id_hex!)}
          >
            Restore access
          </Button>
        ) : null}
        <MenuButton
          variant="quiet"
          icon="more"
          trailingIcon={null}
          label=""
          menuLabel={`Actions for ${entry.remote_team_alias}`}
          aria-label={`Actions for ${entry.remote_team_alias}`}
        >
          {(close) => (
            <>
              <MenuItem reason={memberReason}>Lower role…</MenuItem>
              <MenuItem reason={memberReason}>Remove a member…</MenuItem>
              <MenuItem
                icon="copy"
                title="Copies this admission’s host and operation identifiers, for reporting a failure."
                onClick={() => {
                  close();
                  onCopy(
                    admissionDetails(entry, remoteName),
                    'Admission details copied.',
                  );
                }}
              >
                Copy admission details
              </MenuItem>
              <div className="menu-separator" role="separator" />
              <MenuItem
                danger
                reason={
                  !manageable
                    ? 'Only an Admin or an Owner can remove this admission.'
                    : entry.active
                      ? undefined
                      : 'Restore access before removing this admission.'
                }
                title="Removes the whole admission and rotates this team’s key."
                onClick={() => {
                  close();
                  onRemove(entry);
                }}
              >
                Remove admission…
              </MenuItem>
            </>
          )}
        </MenuButton>
      </span>
    </div>
  );
}

/**
 * The Members tab: people, machines and the teams admitted from other
 * servers in one roster. People and admitted teams share one "Members"
 * heading and one list — a team's row keeps a "team on …" caption naming its
 * server, so it reads as a team rather than a person. (How many people it
 * brings is not drawn: this device holds no roster for a team on another
 * server, and a guessed count would be worse than none.) Machines keep their
 * own heading below, unchanged. What adds to the list — a person on this
 * server, or a team from another server — lives in the page header now, as
 * one "Add people" control.
 */
function MembersTab({
  snapshot,
  store,
  onSheet,
  rosterReason,
  federationReason,
  menuParty,
  failure,
  federationFailure,
  onRetry,
  onRetryFederation,
  onRerun,
  onRemoveAdmission,
  onCopy,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  onSheet: (sheet: Sheet, party?: Party) => void;
  /**
   * Why the roster cannot be added to, and why no group can be admitted —
   * each row's own menu states its own reason, because the two are decided
   * separately and one can apply while the other does not.
   */
  rosterReason?: string;
  federationReason?: string;
  menuParty: string | null;
  failure?: GroupDetailFailure;
  federationFailure?: GroupDetailFailure;
  onRetry: () => void;
  onRetryFederation: () => void;
  onRerun: (operationId: string) => void;
  onRemoveAdmission: (entry: FederationEntry) => void;
  onCopy: (text: string, message: string) => void;
}): ReactNode {
  const manageable = rosterReason === undefined;
  const federationManageable = federationReason === undefined;
  const parties = sortRoster(partiesOf(snapshot, store.id));
  const people = parties.filter(
    (party) => party.party_kind === 'user' && !isMachine(party),
  );
  const machines = parties.filter((party) => isMachine(party));
  const row = (party: Party): ReactNode => (
    <PartyRow
      key={party.party_id_hex}
      snapshot={snapshot}
      store={store}
      party={party}
      manageable={manageable}
      menuOpen={menuParty === party.party_id_hex}
      onSheet={onSheet}
    />
  );
  const { entries, unmatched, ambiguous } = federationFailure
    ? { entries: [], unmatched: [], ambiguous: [] }
    : admittedGroups(snapshot, store);
  const teamCount = entries.length + unmatched.length + ambiguous.length;
  return (
    <div className="roster">
      <SituationBand store={store} tab="people" />
      <SectionLabel>
        Members
        {!failure && !federationFailure ? (
          <span className="count">
            — {plural(memberCountOf(snapshot, store), 'member')}
          </span>
        ) : null}
      </SectionLabel>
      {failure ? (
        <Band
          label="Roster unavailable"
          action={
            failure.retryable ? (
              <Button onClick={onRetry}>Refresh</Button>
            ) : undefined
          }
        >
          {failure.message}
        </Band>
      ) : people.length || teamCount ? (
        <div className="rt bare">
          {people.map(row)}
          {entries.map((entry) => (
            <FederationEntryRow
              key={`${entry.remote_host_id_hex}|${entry.remote_team_id_hex}`}
              snapshot={snapshot}
              entry={entry}
              onRerun={onRerun}
              onRemove={onRemoveAdmission}
              onCopy={onCopy}
              manageable={federationManageable}
            />
          ))}
          {unmatched.map((party) => (
            <TeamPartyRow
              key={party.party_id_hex}
              party={party}
              chip="No admission record"
              chipTitle="The roster lists this team as a member, but no admission record on this device matches it."
            />
          ))}
          {ambiguous.map((party) => (
            <TeamPartyRow
              key={party.party_id_hex}
              party={party}
              chip="Ambiguous admission"
              chipTitle="The roster lists this team once, but several admission records on this device match it, so none of them can be acted on."
            />
          ))}
        </div>
      ) : (
        <div className="callout">
          <span
            className="kico"
            style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
          >
            <Icon name="people" />
          </span>
          <span className="t">
            <b>No members yet.</b>
          </span>
        </div>
      )}
      {/* With the roster unread its rows, the federation's among them, are
          not drawn, so a federation failure with the same sentence would only
          repeat the band above. One with its own sentence still says it. */}
      {federationFailure &&
      (!failure || federationFailure.message !== failure.message) ? (
        <Band
          label="Federation unavailable"
          action={
            federationFailure.retryable ? (
              <Button onClick={onRetryFederation}>Refresh</Button>
            ) : undefined
          }
        >
          {federationFailure.message}
        </Band>
      ) : null}
      {machines.length ? (
        <MemberSection title="Machines" count={`${machines.length} connected`}>
          {machines.map(row)}
        </MemberSection>
      ) : null}
    </div>
  );
}

function FederationRemovalSheet({
  bridge,
  store,
  entry,
  onClose,
  onApplied,
  onMutationError,
}: {
  bridge: Bridge;
  store: Extract<Store, { kind: 'team' }>;
  entry: FederationEntry;
  onClose: () => void;
  onApplied: (message: string) => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const apply = async (): Promise<void> => {
    if (!confirmed || busy || !entry.active) return;
    setBusy(true);
    try {
      const result = await attemptMutation(
        { kind: 'resumable', operation: 'remove-admission' },
        () =>
          bridge.removeFederatedGroup({
            storeId: store.id,
            remoteHostIdHex: entry.remote_host_id_hex,
            remoteTeamIdHex: entry.remote_team_id_hex,
          }),
        () =>
          onApplied(`${entry.remote_team_alias} removed and team keys rotated`),
      );
      if (
        await reportMutationOutcome(result, onMutationError, () => {
          toasts.show('Admission removed. Refresh pending.');
        })
      )
        onClose();
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger
      dismissible={false}
      onClose={onClose}
      glyph={<GroupMark store={store} />}
      title={`Remove ${entry.remote_team_alias} from ${store.name}?`}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={busy || !confirmed || !entry.active}
            onClick={() => void apply()}
          >
            Remove and rekey
          </Button>
        </>
      }
    >
      <p>
        Every member of <b>{entry.remote_team_alias}</b> will lose access to
        this team. Team encryption keys will be updated to block future access.
      </p>
      <Inset>
        <InsetRow label="Remote host">
          <code>{entry.remote_host_id_hex}</code>
        </InsetRow>
        <InsetRow label="Remote team">
          <code title={entry.remote_team_id_hex}>
            {shortId(entry.remote_team_id_hex)}
          </code>
        </InsetRow>
      </Inset>
      <label className="checkline">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        Remove every member of {entry.remote_team_alias} and rotate keys
      </label>
    </SheetDialog>
  );
}

function SettingsTab({
  snapshot,
  store,
  onSheet,
  onNavigate,
  onCopy,
  manageable,
  rekeyOpen,
}: {
  snapshot: AgentSnapshot;
  store: Extract<Store, { kind: 'team' }>;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  manageable: boolean;
  rekeyOpen: boolean;
}): ReactNode {
  const parties = partiesOf(snapshot, store.id);
  const mine = parties.find((party) => party.label === 'you');
  const owner = parties.find(
    (party) => parseRole(party.destination_role)?.kind === 'owner',
  );
  const account = snapshot.accounts.find(
    (candidate) =>
      candidate.alias === store.account && candidate.server === store.server,
  );
  const server = serverOf(snapshot, store.id);
  const removable = manageable
    ? parties
        .map((party, index) => ({ party, index }))
        .filter(({ party }) => canTarget(snapshot, party))
        .sort((left, right) => {
          const rank =
            roleRank(left.party.destination_role) -
            roleRank(right.party.destination_role);
          if (rank) return rank;
          const leftRole = parseRole(left.party.destination_role);
          const rightRole = parseRole(right.party.destination_role);
          const vis =
            (leftRole?.kind === 'member' ? visibilityOf(leftRole) : 0) -
            (rightRole?.kind === 'member' ? visibilityOf(rightRole) : 0);
          if (vis) return vis;
          return left.index - right.index;
        })
        .map(({ party }) => party)
    : [];
  const total = itemsOf(snapshot, store.id).length;
  return (
    <div className="group-settings">
      <SituationBand store={store} tab="settings" />
      <SectionLabel>About this team</SectionLabel>
      <Inset>
        <InsetRow
          label="Name"
          action={
            // Inert rather than disabled: the native web view shows no
            // tooltip on a disabled control, and the title is the only place
            // the reason is stated.
            <Button
              size="sm"
              aria-disabled="true"
              title="Team names cannot be changed after creation."
            >
              Cannot change
            </Button>
          }
        >
          {store.name}
        </InsetRow>
        <InsetRow
          label="Server"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Open server
            </Button>
          }
        >
          {server ? serverDisplayName(server) : store.server}
        </InsetRow>
        <InsetRow label="Your account">
          {account?.username ?? store.account} · {mine ? roleText(mine) : '—'}
        </InsetRow>
        <InsetRow label="Owner">
          {owner ? partyName(owner) : 'No owner designated'}
        </InsetRow>
        <InsetRow
          label="Team ID"
          action={
            <Button
              size="sm"
              icon="copy"
              onClick={() => onCopy(store.team_id_hex)}
            >
              Copy
            </Button>
          }
        >
          <code title={store.team_id_hex}>{shortId(store.team_id_hex)}</code>
        </InsetRow>
      </Inset>
      {store.team_kind === 'adhoc' ? (
        <p className="fn">
          An ad-hoc team has no name on the server and a fixed membership.
        </p>
      ) : (
        <>
          <SectionLabel>Who can join</SectionLabel>
          <Inset>
            <InsetRow label="Join policy">
              Invite only
              <span className="hint">
                An administrator adds members or approves membership requests.
              </span>
            </InsetRow>
          </Inset>
        </>
      )}
      <SectionLabel>Danger</SectionLabel>
      <Inset className="danger settings-inset">
        <InsetRow
          action={
            <MenuButton
              variant="danger"
              label="Remove…"
              menuLabel="Removable members"
              disabled={!removable.length}
              defaultOpen={rekeyOpen}
            >
              {(close) =>
                removable.map((party) => (
                  <button
                    type="button"
                    key={party.party_id_hex}
                    onClick={() => {
                      close();
                      onSheet('remove', party);
                    }}
                  >
                    <span className="t">
                      <b>{partyName(party)}</b>
                      <small>
                        {fmtRole(party.destination_role)} · reads{' '}
                        {readsOf(snapshot, party).length} of {total}
                      </small>
                    </span>
                  </button>
                ))
              }
            </MenuButton>
          }
        >
          <span className="t">
            <b>Remove a team member</b>
            <small>
              Removing anyone rotates the team key and blocks their future
              reads. Copies already downloaded are not erased.
            </small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button
              icon="out"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Servers
            </Button>
          }
        >
          <span className="t">
            <b>
              Reset this device’s state for{' '}
              {server ? serverDisplayName(server) : store.server}
            </b>
            <small>
              Removes locally stored keys and server data from this device.
            </small>
          </span>
        </InsetRow>
      </Inset>
    </div>
  );
}

function inspectResponse(
  snapshot: AgentSnapshot,
  store: Store,
  tab: Tab,
): unknown {
  if (tab === 'settings') return store;
  return {
    people: partiesOf(snapshot, store.id).map((party) => ({
      ...(party.username ? { username: party.username } : {}),
      party_kind: party.party_kind,
      generation: party.generation,
      locally_manageable: party.locally_manageable,
      party_id_hex: party.party_id_hex,
      ...(party.scoped_host_id_hex
        ? { scoped_host_id_hex: party.scoped_host_id_hex }
        : {}),
      source_role: party.source_role,
      destination_role: party.destination_role,
    })),
    federation: snapshot.federation
      .filter((entry) => entry.store === store.id)
      .map((entry) => ({
        remote_profile: entry.remote_profile,
        remote_team_alias: entry.remote_team_alias,
        remote_host_id_hex: entry.remote_host_id_hex,
        remote_team_id_hex: entry.remote_team_id_hex,
        destination: entry.destination,
        ...(entry.operation_id_hex
          ? { operation_id_hex: entry.operation_id_hex }
          : {}),
        active: entry.active,
      })),
  };
}

/**
 * The way out of a creation that finishing cannot fix: a name the server
 * refused, or a saved identity bound to a host, account or device that no
 * longer matches. Only the local record is dropped — nothing here reaches the
 * server — so the sheet first says which of the two cases this record is in,
 * because dropping keys the server already knows about is not the same act as
 * dropping keys it never saw.
 */
export function AbandonGroupSheet({
  snapshot,
  bridge,
  store,
  onClose,
  onRemoved,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: TeamStore;
  onClose: () => void;
  onRemoved: () => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [confirmation, setConfirmation] = useState('');
  const [busy, setBusy] = useState(false);
  const toasts = useToast();
  const serverName = displayServerName(snapshot, store);
  // Only the two phases that prove nothing was accepted are treated as
  // server-clean. Every other phase, including an unknown one, is read as
  // possibly-created, which is the direction that cannot mislead.
  const reachedServer =
    store.creation_phase !== 'preparing' && store.creation_phase !== 'rejected';
  // The removal is already with the agent once it starts, the same as the
  // device removal sheet, so Escape, the backdrop and Cancel stop answering.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetDialog
      danger
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      dismissible={!busy}
      glyph={<GroupMark store={store} />}
      title={`Remove ${store.name}?`}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            danger
            disabled={confirmation !== store.alias || busy}
            onClick={() => {
              setBusy(true);
              void attemptMutation(
                { kind: 'mutation' },
                () => bridge.abandonGroupCreation(store.id),
                onRemoved,
              )
                .then((result) =>
                  reportMutationOutcome(result, onMutationError, () => {
                    toasts.show('Team removed. Refresh pending.');
                  }),
                )
                .finally(() => setBusy(false));
            }}
          >
            Remove team
          </Button>
        </>
      }
    >
      <p>
        {reachedServer
          ? `${store.name} may already have been created on ${serverName}. Removing it from this device will delete its encryption keys, permanently removing access to the team.`
          : `Creation of ${store.name} was not submitted to ${serverName}. Removing it will discard its local draft and keys without affecting the server.`}
      </p>
      <p className="fn">
        This team has no members, channels or items, so nothing else is lost.
        The name is not reserved by this record and can be used again.
      </p>
      <Inset>
        <InsetRow label="Stored as">
          <code>{store.alias}</code>
        </InsetRow>
        <InsetRow label="Confirm">
          <input
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            placeholder={`type ${store.alias}`}
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}

export function GroupSheet({
  snapshot,
  bridge,
  store,
  sheet,
  target,
  onClose,
  onSwitch,
  onInvite,
  onApplied,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Store;
  sheet: Exclude<Sheet, null>;
  target: Party | null;
  onClose: () => void;
  onSwitch: (sheet: Sheet, target?: Party) => void;
  /** Opens the invitation sheet, which belongs to an account, not a group. */
  onInvite?: () => void;
  onApplied: (
    message: string,
    created?: { accountStoreId: StoreRef; teamAlias: string },
  ) => Promise<void>;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const [username, setUsername] = useTabSheetState(
    'group.username',
    SUGGESTED_MEMBER,
  );
  const [visibility, setVisibility] = useTabSheetState('group.visibility', 0);
  const callerParty = partiesOf(snapshot, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const callerRank = callerParty ? roleRank(callerParty.destination_role) : 0;
  const [role, setRole] = useTabSheetState<RoleDto>('group.role', {
    role: 'Member',
    visibility: 0,
  });
  const [busy, setBusy] = useState(false);
  const [name, setName] = useTabSheetState('group.name', SUGGESTED_GROUP);
  const [createKind, setCreateKind] = useTabSheetState<'named' | 'adhoc'>(
    'group.createKind',
    'named',
  );
  const creationAccounts = snapshot.stores.filter(
    (candidate): candidate is AccountStore =>
      candidate.kind === 'account' &&
      storeOperationAvailability(snapshot, candidate, 'teams').available,
  );
  // Creating acts as the account the page acts as: the sheet's own store when
  // that store is an account that can create, else this Mac's first such one.
  const [accountStoreId, setAccountStoreId] = useTabSheetState(
    'group.accountStoreId',
    () =>
      creationAccounts.find((candidate) => candidate.id === store.id)?.id ??
      creationAccounts[0]?.id ??
      '',
  );
  const remotes = snapshot.stores.filter(
    (candidate): candidate is Extract<Store, { kind: 'team' }> =>
      candidate.kind === 'team' &&
      candidate.active &&
      candidate.team_kind === 'named' &&
      storeOperationAvailability(snapshot, candidate, 'federation').available &&
      candidate.server !== store.server,
  );
  const [remoteStoreId, setRemoteStoreId] = useTabSheetState(
    'group.remoteStoreId',
    remotes[0]?.id ?? '',
  );
  const creationAccount = creationAccounts.find(
    (candidate) => candidate.id === accountStoreId,
  );
  const serverName = displayServerName(snapshot, store);
  const teamAlias = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
  // Previews the normalized group name using the client's whitespace rules,
  // ensuring the dialog rejects invalid names before submitting to the agent.
  const reservedName = serverTeamName(name);
  const remote =
    remotes.find((candidate) => candidate.id === remoteStoreId) ?? remotes[0];
  const federationTarget = {
    profile: store.server,
    account: store.account,
    remoteProfile: remote?.server,
  };
  const admissionReason =
    sheet === 'admit'
      ? ((store.kind === 'team'
          ? manageReason(snapshot, store, 'federation')
          : 'Select a named team.') ??
        workflowMessage(
          workflowAvailability(snapshot, 'federate', federationTarget),
        ))
      : undefined;
  // An admission needs the admitted teams, and a read that failed as a whole
  // is recorded under the roster, so the admit sheet consults both.
  const requiredFailure =
    sheet === 'admit'
      ? (groupDetailFailure(snapshot, store.id, 'federation') ??
        groupDetailFailure(snapshot, store.id, 'roster'))
      : ['add', 'demote', 'remove'].includes(sheet)
        ? groupDetailFailure(snapshot, store.id, 'roster')
        : undefined;
  // Adding a person and admitting a group are the two halves of one sheet, so
  // both read from the switch rather than from two separate titles.
  const adding = sheet === 'add' || sheet === 'admit';
  const title = adding
    ? sheet === 'add'
      ? `Add someone to ${store.name}`
      : `Add a team to ${store.name}`
    : sheet === 'demote'
      ? `Lower ${target ? `${partyName(target)}’s` : 'their'} role`
      : sheet === 'remove'
        ? `Remove ${target ? partyName(target) : 'them'} from ${store.name}?`
        : 'Create a team';
  const [demotion, setDemotion] = useState<RoleDto | null>(() =>
    target ? demotionFor(target) : null,
  );
  const currentRole = target ? parseRole(target.destination_role) : null;
  const maxMemberVisibility =
    currentRole?.kind === 'member' ? (currentRole.visibility ?? 0) - 1 : 0;
  useEffect(() => {
    if (sheet === 'demote') setDemotion(target ? demotionFor(target) : null);
  }, [sheet, target]);
  // A username the roster already holds is a local fact, so the sheet refuses
  // it before sending a request the agent would refuse. Everything else a
  // username can be wrong about — unknown, ambiguous — only the server knows,
  // so that refusal is the agent's own sentence, read back here.
  const existing = partiesOf(snapshot, store.id).find(
    (party) =>
      party.party_kind === 'user' &&
      (party.username ?? '').toLowerCase() === username.trim().toLowerCase(),
  );
  const [refused, setRefused] = useState('');
  const toasts = useToast();
  const addRefusal = username.trim()
    ? existing
      ? `${partyName(existing)} is already a member of ${store.name}. Change their role from the Members list instead.`
      : refused
    : '';
  // Once a membership change or a group creation has been sent, the agent
  // holds it. Interrupted operations appear as pending and can resume from this
  // page. Typed input that has not been submitted requires confirmation before
  // dismissal.
  useSheetGuard(
    busy
      ? null
      : sheet === 'add' && username.trim() && username !== SUGGESTED_MEMBER
        ? {
            verdict: 'prompt',
            title: 'Discard this member?',
            body: `${username.trim()} has not been added to ${store.name}.`,
            confirm: 'Discard',
            onConfirm: () => {
              setUsername(SUGGESTED_MEMBER);
              onClose();
            },
          }
        : sheet === 'create' && name.trim() && name !== SUGGESTED_GROUP
          ? {
              verdict: 'prompt',
              title: 'Discard this team?',
              body: `${name.trim()} has not been created.`,
              confirm: 'Discard',
              onConfirm: () => {
                setName(SUGGESTED_GROUP);
                onClose();
              },
            }
          : null,
    !busy && (sheet === 'add' || sheet === 'create'),
  );
  const apply = async (): Promise<void> => {
    if (busy || requiredFailure) return;
    if (sheet === 'add' && existing) return;
    setBusy(true);
    setRefused('');
    const complete = async (created?: {
      accountStoreId: StoreRef;
      teamAlias: string;
    }): Promise<void> => {
      const result = await synchronizeApplied(() =>
        created
          ? onApplied(`${title} completed`, created)
          : onApplied(`${title} completed`),
      );
      if (result.synchronization === 'pending')
        toasts.show(`${title} completed. Refresh pending.`);
      onClose();
    };
    try {
      if (sheet === 'add')
        await bridge.addGroupMember({
          storeId: store.id,
          username: username.trim(),
          destination: role.role === 'Member' ? { ...role, visibility } : role,
        });
      else if (
        sheet === 'demote' &&
        target &&
        canTarget(snapshot, target) &&
        demotion
      )
        await bridge.demoteGroupMember({
          storeId: store.id,
          username: target.username!,
          destination: demotion,
        });
      else if (sheet === 'remove' && target && canTarget(snapshot, target))
        await bridge.removeGroupMember({
          storeId: store.id,
          username: target.username!,
        });
      else if (sheet === 'admit' && remote) {
        if (admissionReason) throw new Error(admissionReason);
        requireWorkflow(snapshot, 'federate', federationTarget);
        await bridge.admitGroup({
          storeId: store.id,
          remoteStoreId: remote.id,
          visibility,
        });
      } else if (sheet === 'create') {
        // Re-resolved from the availability-filtered list at the moment of
        // the write, not from the render that drew the button: the server
        // can lapse while the sheet is open.
        const account = creationAccounts.find(
          (candidate) => candidate.id === accountStoreId,
        );
        if (!account)
          throw new Error('No account store is available for team creation.');
        await bridge.createGroup({
          accountStoreId: account.id,
          teamAlias,
          name: createKind === 'named' ? name : '',
          kind: createKind,
        });
        await complete({ accountStoreId: account.id, teamAlias });
        return;
      } else return;
      await complete();
    } catch (error) {
      // The sheet stays open on a refusal and states it where the field is,
      // in the agent's own words, while the shell reconciles as it always has.
      // The sheet is then the refusal's one surface: the shell does not toast
      // the same sentence beside it.
      if (sheet === 'add') {
        setRefused(normalizeCommandError(error).message);
        await onMutationError(error, { report: false });
      } else await onMutationError(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger={sheet === 'remove'}
      onClose={onClose}
      dismissible={!busy}
      glyph={
        sheet === 'create' || store.kind !== 'team' ? (
          // The group does not exist yet, so it has no mark: a group's color
          // and initial are earned at creation, not previewed over an account.
          <span className="kico md neutral">
            <Icon name="people" />
          </span>
        ) : (
          <GroupMark store={store} />
        )
      }
      title={title}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          {sheet === 'remove' ? (
            <Button
              variant="primary"
              danger
              disabled={
                busy ||
                Boolean(requiredFailure) ||
                !target ||
                !canTarget(snapshot, target)
              }
              onClick={() => void apply()}
            >
              Remove and rekey
            </Button>
          ) : (
            <Button
              variant="primary"
              title={admissionReason}
              disabled={
                busy ||
                Boolean(requiredFailure) ||
                (sheet === 'add' && (!username.trim() || Boolean(existing))) ||
                (sheet === 'demote' &&
                  (!target || !canTarget(snapshot, target) || !demotion)) ||
                (sheet === 'admit' && (!remote || Boolean(admissionReason))) ||
                (sheet === 'create' &&
                  (!teamAlias ||
                    !creationAccount ||
                    (createKind === 'named' && !reservedName)))
              }
              onClick={() => void apply()}
            >
              {sheet === 'add'
                ? `Add ${username.trim() || 'someone'}`
                : sheet === 'demote'
                  ? 'Change role'
                  : sheet === 'admit'
                    ? `Add ${remote?.alias ?? 'team'}`
                    : 'Create team'}
            </Button>
          )}
        </>
      }
    >
      <>
        {requiredFailure ? (
          <Notice
            severity="warn"
            title={`${requiredFailure.source === 'roster' ? 'Roster' : 'Federation'} unavailable`}
          >
            <p>
              {requiredFailure.message} Close this sheet and refresh before
              making changes.
            </p>
          </Notice>
        ) : null}
        {/* Adding a person and admitting another server's group are the two
            ways into this group, so they are one sheet with a switch rather
            than two buttons over two tables. */}
        {adding ? (
          <SegmentedControl
            label="What to add"
            value={sheet}
            items={[
              { id: 'add' as const, label: 'Add a person' },
              { id: 'admit' as const, label: 'Add a team on another server' },
            ]}
            onChange={(next) => {
              if (next !== sheet) onSwitch(next);
            }}
          />
        ) : null}
        {sheet === 'add' ? (
          <>
            <Inset>
              <Field
                label="Username"
                value={username}
                // The agent's refusal was of the username that was sent, so a
                // different one is not refused yet: the sentence goes with it.
                onChange={(next) => {
                  setUsername(next);
                  setRefused('');
                }}
              />
              <InsetRow label="Server" action={<Chip>this team’s server</Chip>}>
                {serverName}
              </InsetRow>
            </Inset>
            {addRefusal ? (
              <p role="alert" className="action-error">
                {addRefusal}
              </p>
            ) : null}
            {onInvite ? (
              <p className="fn">
                No account yet?{' '}
                <Button size="sm" onClick={onInvite}>
                  Invite them to {serverName}…
                </Button>{' '}
                You still add the username yourself when they reply.
              </p>
            ) : null}
            <SectionLabel>Role</SectionLabel>
            <Inset>
              <RadioGroup label={`Role in ${store.name}`}>
                {(['Owner', 'Admin', 'Member'] as const).map((next) => {
                  // An Admin cannot make an Owner. The card keeps its place
                  // and says why rather than vanishing from the list.
                  const refusal =
                    next === 'Owner' && callerRank < 3
                      ? 'Only an Owner can add another Owner.'
                      : '';
                  return (
                    <RadioCard
                      key={next}
                      selected={role.role === next}
                      off={Boolean(refusal)}
                      onSelect={() =>
                        setRole(
                          next === 'Member'
                            ? { role: next, visibility }
                            : { role: next },
                        )
                      }
                      title={next}
                      detail={
                        refusal ||
                        (next === 'Member'
                          ? 'Can only read items.'
                          : next === 'Admin'
                            ? 'Manage vault items and team members, excluding other admins and owners.'
                            : 'Manage vault items, members, permissions, or delete the team.')
                      }
                    />
                  );
                })}
              </RadioGroup>
            </Inset>
            {role.role === 'Member' ? (
              <Inset className="visibility-inset">
                <InsetRow
                  label="Visibility"
                  action={
                    // The band the steppers change reads between them, so the
                    // value is never separated from the controls that set it.
                    <>
                      <Button
                        size="sm"
                        aria-label="Lower the visibility band"
                        disabled={visibility <= VIS_MIN}
                        onClick={() => setVisibility((value) => value - 1)}
                      >
                        −
                      </Button>
                      <span className="vis-value">Visibility {visibility}</span>
                      <Button
                        size="sm"
                        aria-label="Raise the visibility band"
                        disabled={visibility >= VIS_MAX}
                        onClick={() => setVisibility((value) => value + 1)}
                      >
                        +
                      </Button>
                    </>
                  }
                >
                  <span className="hint">
                    Grants access to items matching or exceeding this visibility
                    level. (Default: 0)
                  </span>
                </InsetRow>
              </Inset>
            ) : null}
          </>
        ) : null}
        {sheet === 'demote' ? (
          <>
            <p>
              To grant a higher role, remove the member and add them again with
              the updated role.
            </p>
            <Inset>
              <RadioGroup label="New role">
                {currentRole?.kind === 'owner' ? (
                  <RadioCard
                    selected={demotion?.role === 'Admin'}
                    onSelect={() => setDemotion({ role: 'Admin' })}
                    title="Admin"
                    detail="Full access to team items and permission to manage members."
                  />
                ) : null}
                <RadioCard
                  selected={demotion?.role === 'Member'}
                  disabled={maxMemberVisibility < VIS_MIN}
                  onSelect={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: maxMemberVisibility,
                    })
                  }
                  title={
                    currentRole?.kind === 'member'
                      ? `Member (${maxMemberVisibility})`
                      : 'Member'
                  }
                  detail={
                    currentRole?.kind === 'member'
                      ? `Same role, lower level; ${maxMemberVisibility} at most.`
                      : 'Cannot manage members; can only access items allowed by their member role.'
                  }
                />
                <RadioCard
                  off
                  selected={false}
                  title={
                    <>
                      {target ? roleText(target) : 'Current role'}{' '}
                      <Chip>current</Chip>
                    </>
                  }
                  detail="Current active role."
                />
              </RadioGroup>
            </Inset>
            {demotion?.role === 'Member' ? (
              <div className="vis">
                <Button
                  size="sm"
                  disabled={demotion.visibility <= VIS_MIN}
                  onClick={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: demotion.visibility - 1,
                    })
                  }
                >
                  −
                </Button>
                <span>Visibility {demotion.visibility}</span>
                <Button
                  size="sm"
                  disabled={demotion.visibility >= maxMemberVisibility}
                  onClick={() =>
                    setDemotion({
                      role: 'Member',
                      visibility: demotion.visibility + 1,
                    })
                  }
                >
                  +
                </Button>
              </div>
            ) : null}
          </>
        ) : null}
        {sheet === 'remove' ? (
          <>
            <p>
              Removing blocks future reads and rekeys the team. This user may
              retain a local copy of their current records.
            </p>
            {target ? (
              <Inset>
                <InsetRow action={<RoleChip role={target.destination_role} />}>
                  <span className="t">
                    <b>{partyName(target)}</b>
                    <small>{partySubtitle(target)}</small>
                  </span>
                </InsetRow>
              </Inset>
            ) : null}
            {target && !canTarget(snapshot, target) ? (
              <Notice title={`${partyName(target)} cannot be removed here`}>
                This member cannot be removed here. They are managed by another
                server or account.
              </Notice>
            ) : null}
          </>
        ) : null}
        {sheet === 'admit' ? (
          <>
            <SectionLabel>Team</SectionLabel>
            <Inset>
              {remotes.length ? (
                <RadioGroup label="Team">
                  {remotes.map((group) => {
                    const host = serverOf(snapshot, group.id);
                    return (
                      <RadioCard
                        key={group.id}
                        selected={remote?.id === group.id}
                        onSelect={() => setRemoteStoreId(group.id)}
                        title={group.alias}
                        detail={`on ${host ? serverDisplayName(host) : group.server}`}
                      />
                    );
                  })}
                </RadioGroup>
              ) : (
                <InsetRow label="Team">
                  <span className="dim">No eligible remote team</span>
                </InsetRow>
              )}
            </Inset>
            <SectionLabel>Role for its members</SectionLabel>
            <Inset>
              <InsetRow label="Role">
                <Chip>Member</Chip>
                <span className="dim">for every member</span>
              </InsetRow>
              <InsetRow
                label="Visibility"
                action={
                  // The band the steppers change reads between them, so the
                  // value is never separated from the controls that set it.
                  <>
                    <Button
                      size="sm"
                      aria-label="Lower the visibility band"
                      disabled={visibility <= VIS_MIN}
                      onClick={() => setVisibility((value) => value - 1)}
                    >
                      −
                    </Button>
                    <span className="vis-value">Visibility {visibility}</span>
                    <Button
                      size="sm"
                      aria-label="Raise the visibility band"
                      disabled={visibility >= VIS_MAX}
                      onClick={() => setVisibility((value) => value + 1)}
                    >
                      +
                    </Button>
                  </>
                }
              />
            </Inset>
            <p className="fn">
              If the remote server becomes unreachable, the federated team
              status changes to Inactive and its members cannot access items in
              this team until connectivity is restored.
            </p>
          </>
        ) : null}
        {sheet === 'create' ? (
          <>
            <Inset>
              <Field label="Name" value={name} onChange={setName} />
            </Inset>
            <p className="fn">
              {createKind === 'named' ? (
                reservedName ? (
                  <>
                    Named <code>{reservedName}</code> on the server and stored
                    as <code>{teamAlias}</code>. Neither can be changed once
                    created.
                  </>
                ) : (
                  'Use 3 to 25 letters, numbers, spaces, dots, dashes, or underscores.'
                )
              ) : (
                <>
                  Stored as <code>{teamAlias || '…'}</code>. Cannot be changed
                  once created.
                </>
              )}
            </p>
            <SectionLabel>Server and account</SectionLabel>
            <Inset>
              {creationAccounts.length ? (
                <RadioGroup label="Server and account">
                  {creationAccounts.map((account) => (
                    <RadioCard
                      key={account.id}
                      selected={account.id === accountStoreId}
                      onSelect={() => setAccountStoreId(account.id)}
                      title={displayServerName(snapshot, account)}
                      detail={`as ${snapshot.accounts.find((candidate) => candidate.store === account.id || (candidate.alias === account.account && candidate.server === account.server))?.username ?? account.account} · ${account.account} account`}
                    />
                  ))}
                </RadioGroup>
              ) : (
                <InsetRow label="Account">
                  <span className="dim">No account can create a team.</span>
                </InsetRow>
              )}
            </Inset>
            <details className="dd">
              <summary>Advanced</summary>
              <SectionLabel>Kind</SectionLabel>
              <Inset>
                <RadioGroup label="Kind">
                  {(['named', 'adhoc'] as const).map((kind) => (
                    <RadioCard
                      key={kind}
                      selected={kind === createKind}
                      onSelect={() => setCreateKind(kind)}
                      title={kind === 'named' ? 'Named' : 'Ad-hoc'}
                      detail={
                        kind === 'named'
                          ? 'Has an alias on the server; people can be added and removed over time.'
                          : 'Fixed membership, chosen now, no alias. For a one-off share.'
                      }
                    />
                  ))}
                </RadioGroup>
              </Inset>
            </details>
          </>
        ) : null}
      </>
    </SheetDialog>
  );
}

export function GroupSettingsScreen({
  snapshot,
  bridge,
  location,
  accessNow,
  accessGenerations,
  onNavigate,
  onApplied: onSnapshotApplied,
  onError,
  onMutationError: onSnapshotMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'group-settings' }>;
  /** The shell's availability clock, which the Channels tab shares with Chat. */
  accessNow?: () => number;
  /** The shell's access generation per server, as a channel write reads it. */
  accessGenerations?: ReadonlyMap<string, number>;
  /**
   * `replace` marks a move within this page — the tab strip — which the
   * shell records in place of the last rather than behind it.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const copy = useCopyText(bridge, onError);
  const toasts = useToast();
  // The Channels tab reads the same per-group inbox entry the rail and the
  // Chat column read, so the tab's count and its rows cannot disagree.
  const inbox = useSidebarInbox();
  const initial = stateName();
  const [tab, setTab] = useState<Tab>(
    () => location.tab ?? tabFromState(initial),
  );
  const [sheet, setSheet] = useTabSheetState<Sheet>(
    'groups.sheet',
    () =>
      ['add', 'demote', 'remove', 'admit', 'party-remove'].includes(initial)
        ? initial === 'party-remove'
          ? 'remove'
          : (initial as Sheet)
        : null,
    (value) => value === 'add' || value === 'create',
  );
  // Group invitation creation and request review share one sheet.
  const [inviting, setInviting] = useTabSheetState(
    'groups.inviting',
    initial === 'invite',
    (value) => value,
  );
  const [recoverInvitations, setRecoverInvitations] = useState(false);
  const [addingChannel, setAddingChannel] = useTabSheetState(
    'groups.channel',
    false,
    (value) => value,
  );
  // A New chat sheet whose submission is unresolved cannot be dismissed, and a
  // store switch must not take it away either: a submission has to be settled
  // where it was made.
  const channelUnresolved = useRef(false);
  // One clock for this render, the way the Chat tab reads its own: a memo
  // would freeze the availability decision at the moment the tab mounted, so
  // a check-in that lapses while the tab is open would never be noticed.
  const accessOptions: AvailabilityOptions = accessNow
    ? { nowSeconds: accessNow() }
    : {};
  useEffect(() => {
    setTab(location.tab ?? 'people');
  }, [location.ref, location.tab]);
  const store = storeOf(snapshot, location.ref);
  const parties = useMemo(
    () => (store ? partiesOf(snapshot, store.id) : []),
    [store, snapshot],
  );
  const rosterFailure = store
    ? groupDetailFailure(snapshot, store.id, 'roster')
    : undefined;
  const federationFailure = store
    ? groupDetailFailure(snapshot, store.id, 'federation')
    : undefined;
  // The scene that used to open a member's details panel now opens that row's
  // menu, which is where its actions live.
  const [menuParty] = useState<string | null>(() =>
    initial === 'party'
      ? (parties.find((party) => party.username === 'deploy-bot')
          ?.party_id_hex ?? null)
      : null,
  );
  const [removalTarget, setRemovalTarget] = useState<FederationEntry | null>(
    null,
  );
  // Forgetting an unfinished creation is the one destructive action this page
  // offers before a team exists, so it keeps its own confirmation.
  const [abandoning, setAbandoning] = useState(false);
  // The Requests tab's count is the same shared row the Teams list and the
  // rail read; undefined until it has loaded, and no count is drawn then,
  // the same as a team with nothing pending.
  const requestCounts = useTeamRequestCounts(bridge, snapshot, onError);
  const requestCount = store ? requestCounts.get(store.id) : undefined;
  // An interrupted member addition or role change leaves durable local state
  // that blocks every later membership mutation until it is resumed. Read the
  // account's pending operations for this group so the UI can finish it.
  const groupOperations = useGroupOperationController({
    bridge,
    store: store?.kind === 'team' ? store : null,
    enabled: Boolean(
      store && storeOperationAvailability(snapshot, store, 'teams').available,
    ),
    onSnapshotApplied,
    onSnapshotMutationError,
    onRefreshError: (error) => {
      toasts.show('Change applied. Refresh pending.');
      onError(error);
    },
    onReadError: onError,
  });
  // Both successful changes and reconciled failures can change the durable
  // pending records. Refresh them for every membership action and manual refresh.
  const {
    operations: membershipPending,
    onApplied,
    onMutationError,
    resumeCreation: finishSetup,
    resumeAdmission,
    resumeMembership,
  } = groupOperations;
  const [target, setTarget] = useState<Party | null>(() => {
    if (initial === 'demote')
      return parties.find((party) => party.username === 'priya.n') ?? null;
    if (initial === 'remove')
      return parties.find((party) => party.username === 'dana.okafor') ?? null;
    if (initial === 'party-remove') {
      const party = parties.find(
        (candidate) => candidate.username === 'deploy-bot',
      );
      return party ? { ...party, locally_manageable: false } : null;
    }
    return null;
  });
  // `target` is a snapshot taken when a row's menu was used, and a refresh
  // replaces the roster underneath it. Re-resolved here so a role sheet cannot
  // draft a demotion from a role the party no longer holds. `target` keeps its
  // snapshot's manageability, which a scene may force.
  const freshOf = (party: Party | null): Party | undefined =>
    party
      ? parties.find(
          (candidate) => candidate.party_id_hex === party.party_id_hex,
        )
      : undefined;
  const targetFresh = freshOf(target);
  const targetParty = target
    ? targetFresh
      ? {
          ...target,
          source_role: targetFresh.source_role,
          destination_role: targetFresh.destination_role,
          generation: targetFresh.generation,
        }
      : target
    : null;
  const removalEntry = removalTarget
    ? (snapshot.federation.find(
        (entry) =>
          entry.store === removalTarget.store &&
          entry.active &&
          entry.remote_host_id_hex === removalTarget.remote_host_id_hex &&
          entry.remote_team_id_hex === removalTarget.remote_team_id_hex,
      ) ?? null)
    : null;
  const openSheet = (next: Sheet, party?: Party): void => {
    setTarget(party ?? null);
    setSheet(next);
  };
  // People, machines and admitted groups, each counted once: an admitted group
  // is reported both as a roster party and as a federation entry, and a roster
  // party whose record is missing or ambiguous is still one member.
  const memberCount = useMemo(
    () => (store ? memberCountOf(snapshot, store) : 0),
    [store, snapshot],
  );
  const storeId = store?.id;
  const seenStore = useRef(storeId);
  const [rekeyArmed, setRekeyArmed] = useState(initial === 'rekey-menu');
  useEffect(() => {
    if (seenStore.current === storeId) return;
    if (seenStore.current) {
      setTab('people');
      setSheet(null);
      setTarget(null);
      setRemovalTarget(null);
      setAbandoning(false);
      setRekeyArmed(false);
      setInviting(false);
      // A channel preparation the agent may already hold is the exception:
      // it is settled where it was made, so the sheet stays until it is.
      if (!channelUnresolved.current) setAddingChannel(false);
    }
    seenStore.current = storeId;
  }, [storeId, setAddingChannel, setInviting, setSheet]);

  if (!store || store.kind !== 'team') {
    return (
      <>
        <PageHeader title="Team unavailable" subtitle="" />
        <div className="body">
          <Notice title="This team is no longer available">
            Refresh or choose another team from Teams.
          </Notice>
        </div>
      </>
    );
  }
  const access = storeDescriptionState(snapshot, store, {
    operation: 'metadata',
  });
  const unavailable = access !== 'normal' && access !== 'setup-incomplete';
  const inactive = store.active === false;
  const rosterReason = manageReason(snapshot, store, 'roster');
  const federationReason = manageReason(snapshot, store, 'federation');
  const canReadRoster = storeOperationAvailability(
    snapshot,
    store,
    'teams',
  ).available;
  const canManageRoster = rosterReason === undefined;
  const named = store.team_kind === 'named';
  const serverName = displayServerName(snapshot, store);
  // The header's own chip: server, then the member count, each dropped
  // rather than guessed when it is not known. The count is silent while the
  // roster or the admitted teams could not be read, the same as the Members
  // tab's own count.
  const headerChip = [
    serverName,
    !canReadRoster || rosterFailure || federationFailure
      ? null
      : plural(memberCount, 'member'),
  ]
    .filter(Boolean)
    .join(' · ');
  // The channels this group's inbox entry holds, or `undefined` while the
  // service has not answered for it. A hidden conversation is listed on the
  // tab but not counted on the strip, the way New chat counts.
  const channelEntry = inbox.get(store.id);
  const channelCount = channelEntry?.data
    ? listChannels(
        channelEntry.data.channels,
        channelEntry.data.conversations,
      ).filter((option) => !option.conversation?.hidden).length
    : undefined;
  // The account the invitation is sent as: the one that holds this group.
  const groupAccount = snapshot.stores.find(
    (candidate): candidate is AccountStore =>
      candidate.kind === 'account' &&
      candidate.server === store.server &&
      candidate.account === store.account,
  );
  const inviteSheet = inviting ? (
    <InvitationPanel
      bridge={bridge}
      recover={recoverInvitations}
      teamId={store.team_id_hex}
      profile={store.server}
      account={store.account}
      teamAlias={store.alias}
      presentation={{
        title: 'Invitations and requests',
        onClose: () => {
          setInviting(false);
          setRecoverInvitations(false);
        },
      }}
      onComplete={() => onApplied('Team requests updated')}
    />
  ) : null;
  return (
    <>
      <div className="ghero">
        <button
          type="button"
          className="back"
          title="Teams home"
          aria-label="Teams home"
          onClick={() => onNavigate({ kind: 'teams' })}
        >
          <Icon name="chev" />
        </button>
        <GroupMark store={store} size="big" />
        <div className="t">
          <h1>
            <span>{store.name}</span>
            {inactive ? <Chip tone="warn">Inactive</Chip> : null}
          </h1>
          {/* The role this account holds here is not repeated in the header;
              it already reads on this account's own row under Members. */}
          <div className="sub">
            {headerChip ? <Chip>{headerChip}</Chip> : null}
          </div>
        </div>
        <div className="header-action">
          {/* A person by username here, or a team from another server: one
              control, two choices, each stating its own reason when it does
              not apply, rather than three separate buttons for what is really
              one decision. Ad-hoc teams have a fixed membership and get
              neither. */}
          {named && !unavailable && !inactive ? (
            <MenuButton
              variant="primary"
              icon="plus"
              label="Add people"
              menuLabel="Add people"
            >
              {(close) => (
                <>
                  <MenuItem
                    icon="person"
                    reason={rosterReason}
                    onClick={() => {
                      close();
                      openSheet('add');
                    }}
                  >
                    <span className="menu-choice">
                      <b>A user</b>
                      <small>Invite by their username</small>
                    </span>
                  </MenuItem>
                  <MenuItem
                    icon="people"
                    reason={federationReason}
                    onClick={() => {
                      close();
                      openSheet('admit');
                    }}
                  >
                    <span className="menu-choice">
                      <b>A team from another server</b>
                      <small>Add a remote team via federation</small>
                    </span>
                  </MenuItem>
                  <div className="menu-separator" role="separator" />
                  {/* The third way in, kept apart from the two that name a
                      party: an invitation is issued now and answered later,
                      by someone this device cannot name yet. */}
                  <MenuItem
                    icon="door"
                    reason={rosterReason}
                    onClick={() => {
                      close();
                      setInviting(true);
                    }}
                  >
                    <span className="menu-choice">
                      <b>By invitation…</b>
                      <small>
                        Create an invitation to send, and review pending
                        operations.
                      </small>
                    </span>
                  </MenuItem>
                </>
              )}
            </MenuButton>
          ) : null}
          {/* The group ID and the vault are local facts, so the menu stays even
              while the server is out of reach; only what needs the server is
              disabled, with the reason in the item. */}
          <MenuButton
            variant="quiet"
            icon="more"
            trailingIcon={null}
            label=""
            menuLabel="Team actions"
            // Named for the team it acts on: several triggers on this page
            // read "More" otherwise, and none of them says what it acts on.
            title={`Actions for ${store.name}`}
            aria-label={`Actions for ${store.name}`}
          >
            {(close) => (
              <>
                <MenuItem
                  icon="again"
                  reason={
                    unavailable
                      ? `Restore access to ${displayServerName(snapshot, store)} first.`
                      : inactive
                        ? 'Finish setting up this team first.'
                        : undefined
                  }
                  onClick={() => {
                    close();
                    void onApplied('Team refreshed').catch(onError);
                  }}
                >
                  Refresh team
                </MenuItem>
                <MenuItem
                  icon="copy"
                  onClick={() => {
                    close();
                    void copy(store.team_id_hex, 'Team ID copied.');
                  }}
                >
                  Copy team ID
                </MenuItem>
                <MenuItem
                  icon="out"
                  onClick={() => {
                    close();
                    onNavigate({ kind: 'store', ref: store.id });
                  }}
                >
                  Open in vault
                </MenuItem>
              </>
            )}
          </MenuButton>
        </div>
      </div>
      {unavailable ? (
        <StoreAccessTakeover
          operation="metadata"
          snapshot={snapshot}
          store={store}
          noHeader
          variant="band"
          onOpenServer={(profile) =>
            onNavigate({ kind: 'settings', section: 'servers', profile })
          }
          onFinishSetup={finishSetup}
        />
      ) : inactive ? (
        // A group with incomplete setup has no members, channels, items, or
        // tabs. Show its status and recovery actions instead.
        <IncompleteGroupPage
          snapshot={snapshot}
          store={store}
          onFinish={finishSetup}
          onRemove={() => setAbandoning(true)}
          onCopyId={() => void copy(store.team_id_hex, 'Team ID copied.')}
          onNavigate={onNavigate}
        />
      ) : (
        <>
          {/* The active invitation panel owns its reads while it is open. */}
          {canManageRoster && !inviting ? (
            <InvitationRecovery
              bridge={bridge}
              store={store}
              onError={onError}
              onReview={() => {
                setRecoverInvitations(true);
                setInviting(true);
              }}
            />
          ) : null}
          {membershipPending.map((operation) => (
            <Band
              key={`${operation.kind}:${operation.target ?? ''}`}
              // This one is mounted by what the reader did — a change that
              // stopped partway, read back after the action — so it announces.
              live
              label="Finish a pending membership change"
              action={
                <Button
                  size="sm"
                  variant="primary"
                  aria-label={
                    operation.kind === 'team-member-addition'
                      ? `Resume adding ${operation.target ?? 'member'}`
                      : 'Resume the role change'
                  }
                  onClick={() => resumeMembership(operation)}
                >
                  Resume
                </Button>
              }
            >
              {operation.kind === 'team-member-addition'
                ? `FOKS stopped partway through adding ${operation.target ?? 'a member'}.`
                : 'FOKS stopped partway through a role change.'}{' '}
              Finish the pending change before adding, removing, or changing
              anyone else.
            </Band>
          ))}
          <Tabs
            label="Team sections"
            idBase={GROUP_TABS}
            value={tab}
            onChange={(next) => {
              setTab(next);
              // Selection follows focus on the strip, so walking it with the
              // arrows is one navigation per key. A tab is a place within this
              // page rather than a page of its own, so each one replaces the
              // last instead of stacking a step behind the reader.
              onNavigate(
                { kind: 'group-settings', ref: store.id, tab: next },
                { replace: true },
              );
            }}
            items={GROUP_SETTINGS_TABS.filter(
              (id) => id !== 'requests' || canManageRoster,
            ).map((id) =>
              id === 'people'
                ? {
                    id,
                    label: 'Members',
                    ...(!canReadRoster || rosterFailure || federationFailure
                      ? {}
                      : { count: memberCount }),
                  }
                : id === 'channels'
                  ? {
                      id,
                      label: 'Channels',
                      // The count is the service's, so it is drawn only once
                      // the service has answered for this group.
                      ...(channelCount === undefined
                        ? {}
                        : { count: channelCount }),
                    }
                  : id === 'files'
                    ? {
                        id,
                        label: 'Files',
                        ...(storeOperationAvailability(snapshot, store, 'vault')
                          .available
                          ? { count: itemCountOf(snapshot, store) }
                          : {}),
                      }
                    : id === 'requests'
                      ? {
                          id,
                          label: 'Requests',
                          // No pending request reads as no count, not zero:
                          // the tab is drawn the same as Channels and Files
                          // before their own counts are known.
                          ...(requestCount ? { count: requestCount } : {}),
                        }
                      : { id, label: 'Settings' },
            )}
          />
          {/* Fixed IDs associate each tab button with its tabpanel. */}
          <div
            className="body"
            role="tabpanel"
            id={tabPanelId(GROUP_TABS, tab)}
            aria-labelledby={tabId(GROUP_TABS, tab)}
          >
            <div className="groups-wrap">
              {tab === 'people' && !canReadRoster ? (
                <StoreAccessTakeover
                  snapshot={snapshot}
                  store={store}
                  operation="teams"
                  noHeader
                  variant="band"
                  onOpenServer={(profile) =>
                    onNavigate({
                      kind: 'settings',
                      section: 'servers',
                      profile,
                    })
                  }
                  onFinishSetup={finishSetup}
                />
              ) : tab === 'people' ? (
                <MembersTab
                  snapshot={snapshot}
                  store={store}
                  onSheet={openSheet}
                  {...(rosterReason ? { rosterReason } : {})}
                  {...(federationReason ? { federationReason } : {})}
                  menuParty={menuParty}
                  failure={rosterFailure}
                  federationFailure={federationFailure}
                  onRetry={() => void onApplied('Refreshing team members…')}
                  onRetryFederation={() =>
                    void onApplied('Refreshing external teams…')
                  }
                  onRerun={resumeAdmission}
                  onRemoveAdmission={setRemovalTarget}
                  onCopy={(text, message) => void copy(text, message)}
                />
              ) : tab === 'channels' ? (
                <ChannelsTab
                  snapshot={snapshot}
                  store={store}
                  accessOptions={accessOptions}
                  onNavigate={onNavigate}
                  onAddChannel={() => setAddingChannel(true)}
                />
              ) : tab === 'files' ? (
                <FilesTab
                  snapshot={snapshot}
                  store={store}
                  onNavigate={onNavigate}
                />
              ) : tab === 'settings' ? (
                <SettingsTab
                  snapshot={snapshot}
                  store={store}
                  onSheet={openSheet}
                  onNavigate={onNavigate}
                  onCopy={(text) => void copy(text, 'Team ID copied.')}
                  manageable={canManageRoster}
                  rekeyOpen={rekeyArmed}
                />
              ) : null}
              {/* Mounted for the life of the page, not just while the
                  Requests tab is open, so its count is known before the
                  reader ever switches to it; only its visibility follows the
                  tab. */}
              {canManageRoster ? (
                <div hidden={tab !== 'requests'}>
                  <InvitationPanel
                    key={store.id}
                    bridge={bridge}
                    profile={store.server}
                    account={store.account}
                    teamAlias={store.alias}
                    requestsOnly
                    onComplete={() => onApplied('Team requests updated')}
                  />
                </div>
              ) : null}
              {/* The raw response belongs to the two tabs it is the response
                  for: the roster under Members, the store under Settings. */}
              {tab === 'settings' ||
              (tab === 'people' && !rosterFailure && !federationFailure) ? (
                <Toggle label="Inspect response">
                  <pre>
                    {JSON.stringify(
                      inspectResponse(snapshot, store, tab),
                      null,
                      1,
                    )}
                  </pre>
                </Toggle>
              ) : null}
            </div>
          </div>
          {removalEntry ? (
            <FederationRemovalSheet
              bridge={bridge}
              store={store}
              entry={removalEntry}
              onClose={() => setRemovalTarget(null)}
              onApplied={onApplied}
              onMutationError={onMutationError}
            />
          ) : null}
          {sheet ? (
            <GroupSheet
              // Adding a person and admitting a group are two halves of one
              // sheet, but not one set of answers: the band, the role and the
              // refusal belong to the half they were given on, so switching
              // starts the other half rather than inheriting them.
              key={sheet}
              snapshot={snapshot}
              bridge={bridge}
              store={store}
              sheet={sheet}
              target={targetParty}
              onClose={() => setSheet(null)}
              onSwitch={openSheet}
              onInvite={
                groupAccount
                  ? () => {
                      setSheet(null);
                      setInviting(true);
                    }
                  : undefined
              }
              onApplied={onApplied}
              onMutationError={onMutationError}
            />
          ) : null}
          {/* Creating a channel is the New chat sheet's own create step, aimed
              at this group: one preparation, recovered where it was made. */}
          {addingChannel ? (
            <NewChatSheet
              snapshot={snapshot}
              bridge={bridge}
              team={store.id}
              accessOptions={accessOptions}
              {...(accessNow ? { accessNow } : {})}
              {...(accessGenerations ? { accessGenerations } : {})}
              onUnresolved={(unresolved) => {
                channelUnresolved.current = unresolved;
              }}
              onClose={() => {
                channelUnresolved.current = false;
                setAddingChannel(false);
              }}
              onOpen={(ref, channel) => {
                channelUnresolved.current = false;
                setAddingChannel(false);
                onNavigate({
                  kind: 'chat',
                  ref,
                  ...(channel ? { channel } : {}),
                });
              }}
            />
          ) : null}
        </>
      )}
      {/* Kept outside the tabs: the team it removes has none. */}
      {abandoning ? (
        <AbandonGroupSheet
          snapshot={snapshot}
          bridge={bridge}
          store={store}
          onClose={() => setAbandoning(false)}
          onRemoved={async () => {
            setAbandoning(false);
            // The page's own store is gone, so the list it came from is the
            // only place left to stand.
            onNavigate({ kind: 'teams' });
            await onApplied(`${store.name} removed`);
          }}
          onMutationError={onMutationError}
        />
      ) : null}
      {inviteSheet}
    </>
  );
}
