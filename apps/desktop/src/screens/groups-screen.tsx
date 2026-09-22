import { federatedTeamMembers, memberCountOf } from './team-members';
import { useGroupOperationController } from './groups/operation-controller';
import { AddPersonSheet } from './groups/add-person-sheet';
import { AddTeamSheet } from './groups/add-team-sheet';
import { CreateTeamSheet } from './groups/create-team-sheet';
import { LowerRoleSheet } from './groups/lower-role-sheet';
import { RemoveMemberSheet } from './groups/remove-member-sheet';
import {
  canTarget,
  demotionFor,
  fmtRole,
  partySubtitle,
  RoleChip,
  roleText,
} from './groups/sheet-support';
import type { GroupSheetBaseProps } from './groups/sheet-support';
import {
  attemptMutation,
  reportMutationOutcome,
} from '../commands/command-policy';
import { useTabSheetState } from '../navigation-guard';
import { InvitationRecovery } from '../components/invitation-recovery';
import { InviteNewUserSheet } from '../components/invite-new-user-sheet';
import { InvitationActivitySheet } from '../components/invitation-activity-sheet';
import { MembershipRequests } from '../components/membership-requests';
import { useTeamRequestCounts } from '../operation-queries';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  MenuButton,
  MenuItem,
  Notice,
  SectionLabel,
  SheetDialog,
  tabId,
  tabPanelId,
  Tabs,
} from '../components';
import {
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
  roleRank,
  serverDisplayLabel,
  serverDisplayLabelForStore as displayServerName,
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
  TeamStore,
  AgentSnapshot,
} from '../model';
import type { Bridge } from '../bridge';
import { useSidebarInbox } from '../chat/inbox-provider';
import { listChannels } from '../chat/presentation';
import { GROUP_SETTINGS_TABS } from '../location';
import type { GroupSettingsTab, Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { markProfileRostersStale } from '../roster-staleness';
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
export type GroupSheetKind =
  'add' | 'demote' | 'remove' | 'add-team' | 'create';
type Sheet = GroupSheetKind | null;

export { serverTeamName } from './groups/sheet-support';
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
    return 'Members of an federated team are managed on their own server and cannot be changed or removed one by one.';
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
      {machine ? <Icon name="terminal" /> : name.slice(0, 1).toUpperCase()}
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
    // Only people and machines reach this row, and their access is their
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
          icon="ellipsis"
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
 * What a federated team's row keeps out of its caption: the server, the host
 * this device knows it by, and the operation that added it. One block of
 * text, so a failure can be reported without reading it off the screen.
 */
function federatedMembershipDetails(
  entry: FederationEntry,
  remoteName: string,
): string {
  return [
    `team ${entry.remote_team_alias} on ${remoteName}`,
    `host ${entry.remote_host_id_hex}`,
    `team id ${entry.remote_team_id_hex}`,
    ...(entry.operation_id_hex ? [`operation ${entry.operation_id_hex}`] : []),
  ].join('\n');
}

/**
 * A roster party that stands for another group whose membership record cannot be
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
 * One federated team from another server, drawn as a member row: it keeps its
 * own facts — the membership state, the host id and the operation id, because
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
    (server) => server.profileName === entry.remote_profile,
  );
  const remoteName = remoteServer
    ? serverDisplayLabel(remoteServer)
    : entry.remote_profile;
  const memberReason =
    'Every member of a federated team holds the role shown on its row. They are managed on their own server and cannot be changed or removed one by one.';
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
              failed member addition, so they are in the row's menu instead. */}
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
            icon="refresh"
            disabled={!manageable}
            title={
              manageable
                ? undefined
                : 'Only an Admin or an Owner can restore this federated membership.'
            }
            onClick={() => onRerun(entry.operation_id_hex!)}
          >
            Restore access
          </Button>
        ) : null}
        <MenuButton
          variant="quiet"
          icon="ellipsis"
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
                title="Copies this federated membership’s host and operation identifiers, for reporting a failure."
                onClick={() => {
                  close();
                  onCopy(
                    federatedMembershipDetails(entry, remoteName),
                    'Membership details copied.',
                  );
                }}
              >
                Copy membership details
              </MenuItem>
              <div className="menu-separator" role="separator" />
              <MenuItem
                danger
                reason={
                  !manageable
                    ? 'Only an Admin or an Owner can remove this federated membership.'
                    : entry.active
                      ? undefined
                      : 'Restore access before removing this federated membership.'
                }
                title="Removes the federated team and rotates this team’s key."
                onClick={() => {
                  close();
                  onRemove(entry);
                }}
              >
                Remove federated team…
              </MenuItem>
            </>
          )}
        </MenuButton>
      </span>
    </div>
  );
}

/**
 * The Members tab: people, machines and the federated teams from other
 * servers in one roster. People and federated teams share one "Members"
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
  onRemoveFederatedMembership,
  onCopy,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  onSheet: (sheet: Sheet, party?: Party) => void;
  /**
   * Why the roster cannot be added to, and why no federated team can be added —
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
  onRemoveFederatedMembership: (entry: FederationEntry) => void;
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
    : federatedTeamMembers(snapshot, store);
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
              onRemove={onRemoveFederatedMembership}
              onCopy={onCopy}
              manageable={federationManageable}
            />
          ))}
          {unmatched.map((party) => (
            <TeamPartyRow
              key={party.party_id_hex}
              party={party}
              chip="No membership record"
              chipTitle="The roster lists this team as a member, but no membership record on this device matches it."
            />
          ))}
          {ambiguous.map((party) => (
            <TeamPartyRow
              key={party.party_id_hex}
              party={party}
              chip="Ambiguous membership"
              chipTitle="The roster lists this team once, but several membership records on this device match it, so none of them can be acted on."
            />
          ))}
        </div>
      ) : (
        <div className="callout">
          <span
            className="kico"
            style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
          >
            <Icon name="users" />
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
  /**
   * Takes the profile, so the read back after a removal is that profile's
   * rather than every profile's. The parent already passes a handler with
   * this signature; the narrower declaration here only hid the argument.
   */
  onApplied: (message: string, profile?: string) => Promise<void>;
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
        { kind: 'resumable', operation: 'remove-federated-membership' },
        () => {
          // Removing a federated team rekeys the team, which moves its chain. The
          // read back must not reuse the roster it holds even if the agent's
          // catalog has not yet caught up with that sequence.
          markProfileRostersStale(store.server);
          return bridge.removeFederatedGroup({
            storeId: store.id,
            remoteHostIdHex: entry.remote_host_id_hex,
            remoteTeamIdHex: entry.remote_team_id_hex,
          });
        },
        () =>
          onApplied(
            `${entry.remote_team_alias} removed and team keys rotated`,
            store.server,
          ),
      );
      if (
        await reportMutationOutcome(result, onMutationError, () => {
          toasts.show('Federated team removed. Refresh pending.');
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
  onRange,
  manageable,
  rekeyOpen,
}: {
  snapshot: AgentSnapshot;
  store: Extract<Store, { kind: 'team' }>;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onRange?: (raise: boolean) => void;
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
                  section: 'account',
                  profile: store.server,
                })
              }
            >
              Open server
            </Button>
          }
        >
          {server ? serverDisplayLabel(server) : store.server}
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
      {manageable && onRange && store.team_kind === 'named' ? (
        <>
          <SectionLabel>Team nesting order</SectionLabel>
          <p className="fn">
            A team joining another team must sit below it in the hierarchy.
            These controls move {store.name}’s position and validate it against
            existing memberships.
          </p>
          <div className="btns">
            <Button onClick={() => onRange(false)}>
              Lower this team’s range
            </Button>
            <Button onClick={() => onRange(true)}>
              Raise this team’s range
            </Button>
          </div>
        </>
      ) : null}
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
              icon="externalLink"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
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
              {server ? serverDisplayLabel(server) : store.server}
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
                () => {
                  // The team and its roster leave this profile's catalog.
                  markProfileRostersStale(store.server);
                  return bridge.abandonGroupCreation(store.id);
                },
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

/** Selects the sheet component for a team-management action. */
export function GroupSheet({
  sheet,
  target,
  onInvite,
  ...base
}: GroupSheetBaseProps & {
  sheet: Exclude<Sheet, null>;
  target: Party | null;
  /** Starts the invitation flow for someone without a FOKS account. */
  onInvite?: () => void;
}): ReactNode {
  switch (sheet) {
    case 'add':
      return <AddPersonSheet {...base} onInvite={onInvite} />;
    case 'add-team':
      return <AddTeamSheet {...base} />;
    case 'demote':
      return <LowerRoleSheet {...base} target={target} />;
    case 'remove':
      return <RemoveMemberSheet {...base} target={target} />;
    case 'create':
      return <CreateTeamSheet {...base} />;
  }
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
  onApplied: (message: string, profile?: string) => Promise<void>;
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
      ['add', 'demote', 'remove', 'add-team', 'party-remove'].includes(initial)
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
  const [reviewing, setReviewing] = useState(false);
  const [addingChannel, setAddingChannel] = useTabSheetState(
    'groups.channel',
    false,
    (value) => value,
  );
  // A New chat sheet whose submission is unresolved cannot be dismissed, and a
  // store switch must not take it away either: a submission has to be settled
  // where it was made.
  const channelUnresolved = useRef(false);
  // One clock for this render, the way the Chat tab reads its own: a cache
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
    resumeFederatedMemberAdd,
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
  // People, machines and federated teams, each counted once: a federated team
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
      setReviewing(false);
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
          {/* A store that is gone is the same class of problem as an account
              that is gone, and takes the same treatment: a danger row alert
              whose body is a paragraph like every other notice's. */}
          <Notice severity="crit" title="Team no longer available">
            <p>Refresh the page, or select another team from the Teams tab.</p>
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
  // The header's badges: server, then the member count, each dropped
  // rather than guessed when it is not known. The count is silent while the
  // roster or the federated teams could not be read, the same as the Members
  // tab's own count.
  const headerChips = [
    serverName,
    !canReadRoster || rosterFailure || federationFailure
      ? null
      : plural(memberCount, 'member'),
  ].filter(Boolean);
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
  const approver = snapshot.accounts.find(
    (candidate) =>
      candidate.alias === store.account && candidate.server === store.server,
  )?.username;
  const inviteSheet = inviting ? (
    <InviteNewUserSheet
      bridge={bridge}
      team={store}
      serverLabel={serverName}
      approver={approver}
      onClose={() => setInviting(false)}
      onComplete={() => onApplied('Invitation created', store.server)}
    />
  ) : null;
  const activitySheet = reviewing ? (
    <InvitationActivitySheet
      bridge={bridge}
      team={store}
      onClose={() => setReviewing(false)}
      onComplete={() => onApplied('Team requests updated', store.server)}
    />
  ) : null;
  const changeRange = (raise: boolean): void => {
    void bridge
      .invitation(
        store.server,
        store.account,
        { action: 'range', team_alias: store.alias, raise },
        null,
      )
      .then(() =>
        toasts.show(
          raise
            ? `${store.name}’s range raised.`
            : `${store.name}’s range lowered.`,
        ),
      )
      .catch((error: unknown) => onMutationError(error));
  };
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
          <Icon name="chevronDown" />
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
            {headerChips.map((label, index) => (
              <Chip key={index}>{label}</Chip>
            ))}
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
                    icon="user"
                    reason={rosterReason}
                    onClick={() => {
                      close();
                      openSheet('add');
                    }}
                  >
                    <span className="menu-choice">
                      <b>Add FOKS user…</b>
                      <small>Someone with an account on {serverName}</small>
                    </span>
                  </MenuItem>
                  <MenuItem
                    icon="users"
                    reason={federationReason}
                    onClick={() => {
                      close();
                      openSheet('add-team');
                    }}
                  >
                    <span className="menu-choice">
                      <b>Add FOKS team…</b>
                      <small>A team from another server</small>
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
                      <b>Invite new user…</b>
                      <small>Create an invitation to share</small>
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
            icon="ellipsis"
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
                  icon="refresh"
                  reason={
                    unavailable
                      ? `Restore access to ${displayServerName(snapshot, store)} first.`
                      : inactive
                        ? 'Finish setting up this team first.'
                        : undefined
                  }
                  onClick={() => {
                    close();
                    // Nothing was written, so the team's chain sequence is
                    // unchanged and the profile read would reuse the roster it
                    // already holds. The user asked for a roster read, so say
                    // so; without this the button does nothing visible.
                    markProfileRostersStale(store.server);
                    void onApplied('Team refreshed', store.server).catch(
                      onError,
                    );
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
          variant="band"
          onOpenServer={(profile) =>
            onNavigate({ kind: 'settings', section: 'account', profile })
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
          {canManageRoster && !reviewing ? (
            <InvitationRecovery
              bridge={bridge}
              store={store}
              onError={onError}
              onReview={() => setReviewing(true)}
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
                  variant="band"
                  onOpenServer={(profile) =>
                    onNavigate({
                      kind: 'settings',
                      section: 'account',
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
                  onRetry={() => {
                    // As with "Refresh team": these retry a roster read that
                    // failed or looked stale, and no write moved the chain, so
                    // the reuse has to be waived explicitly.
                    markProfileRostersStale(store.server);
                    void onApplied('Refreshing team members…', store.server);
                  }}
                  onRetryFederation={() => {
                    markProfileRostersStale(store.server);
                    void onApplied('Refreshing external teams…', store.server);
                  }}
                  onRerun={resumeFederatedMemberAdd}
                  onRemoveFederatedMembership={setRemovalTarget}
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
                  onRange={changeRange}
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
                  <MembershipRequests
                    key={store.id}
                    bridge={bridge}
                    team={store}
                    serverLabel={serverName}
                    servers={snapshot.servers}
                    onComplete={() =>
                      onApplied('Team requests updated', store.server)
                    }
                    onInvite={() => setInviting(true)}
                    onActivity={() => setReviewing(true)}
                  />
                </div>
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
              // Remount when the sheet changes to reset its visibility band,
              // selected role, and validation errors.
              key={sheet}
              snapshot={snapshot}
              bridge={bridge}
              store={store}
              sheet={sheet}
              target={targetParty}
              onClose={() => setSheet(null)}
              onInvite={
                groupAccount
                  ? () => {
                      setSheet(null);
                      setInviting(true);
                    }
                  : undefined
              }
              onApplied={(message, options) =>
                onApplied(message, options?.profile)
              }
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
            await onApplied(`${store.name} removed`, store.server);
          }}
          onMutationError={onMutationError}
        />
      ) : null}
      {inviteSheet}
      {activitySheet}
    </>
  );
}
