import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
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
  SheetDialog,
  tabId,
  tabPanelId,
  Tabs,
  Toggle,
} from '../components';
import {
  actionableGroupMember,
  catalog,
  canCreateInStore,
  groupDetailFailure,
  hue,
  isMachine,
  parseRole,
  partiesOf,
  partyName,
  peopleLabel,
  readersOf,
  roleChipLabel,
  roleName,
  roleRank,
  serverOf,
  storeDescriptionState,
  storeOf,
  storeReadable,
  visibilityOf,
} from '../model';
import type {
  Account,
  AccountStore,
  FederationEntry,
  GroupDetailFailure,
  Item,
  Party,
  Server,
  Store,
  StoreRef,
  TeamStore,
  AgentSnapshot,
} from '../model';
import { enqueueProfileWork } from '../bridge';
import type { Bridge, PendingOperation } from '../bridge';
import type { RoleDto } from '../bridge';
import type { GroupSettingsTab, Location } from '../location';
import type { MutationFailureHandler } from '../mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { accountSubtitle } from './settings-screen';
import { StoreAccessTakeover } from './store-access';
import { useToast } from '/kit/toasts';

type Tab = GroupSettingsTab;
export type GroupSheetKind =
  'invite' | 'add' | 'demote' | 'remove' | 'admit' | 'create';
type Sheet = GroupSheetKind | null;

const VIS_MIN = -32768;
const VIS_MAX = 32767;
/** The base the group page's tab and panel ids are derived from. */
const GROUP_TABS = 'group-sections';
/** Why the join policy cannot be changed: no command sets one. */
const JOIN_POLICY_REASON =
  'No command sets a join policy, opens a group to a server, or accepts a join request, so there is nothing to choose here yet.';
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

/** One notation for a role everywhere on this page: "Owner", "Member (0)". */
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

function oxfordOr(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  if (names.length === 2) return `${names[0]} or ${names[1]}`;
  return `${names.slice(0, -1).join(', ')}, or ${names[names.length - 1]}`;
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
  return name === 'settings' || name === 'danger' || name === 'rekey-menu'
    ? 'settings'
    : 'people';
}

/**
 * A group's mark: its initial over a colour derived from its name, or the
 * inactive grey. One mark for a group everywhere it is listed, so the list row
 * and the page it opens agree.
 */
export function GroupMark({
  store,
  size = 'md',
}: {
  store: Store;
  /** `sm` is the list row's 26px mark; `md` a sheet's; `big` the page hero's. */
  size?: 'sm' | 'md' | 'big';
}): ReactNode {
  return (
    <span
      className={['kico', size === 'sm' ? '' : size, 'group']
        .filter(Boolean)
        .join(' ')}
      // The initial stands for the name beside it; a row that read "E
      // Engineering" would say the name one and a half times.
      aria-hidden="true"
      style={{
        background:
          store.kind === 'team' && store.active === false
            ? 'var(--c-none)'
            : hue(store.name),
      }}
    >
      {store.name.slice(0, 1)}
    </span>
  );
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
 * What discovery needs to run for one account, resolved from its exact
 * identity.
 *
 * `Account.store` is the identity; the alias is profile-local and two
 * profiles can each hold `personal`, so resolving by alias would sooner or
 * later authenticate the wrong account. Every relationship is re-checked
 * here and any inconsistency fails closed, because the caller is about to
 * write durable local bindings with whatever this returns.
 */
export interface DiscoveryContext {
  account: Account;
  store: AccountStore;
  server: Server;
  /** Discovery reads the server, so a stopped server cannot be checked. */
  available: boolean;
}

export function discoveryContext(
  snapshot: AgentSnapshot,
  ref: StoreRef | null,
): DiscoveryContext | null {
  if (!ref) return null;
  const account = snapshot.accounts.find(
    (candidate) => candidate.store === ref,
  );
  if (!account) return null;
  const store = snapshot.stores.find(
    (candidate) => candidate.id === account.store,
  );
  if (!store || store.kind !== 'account') return null;
  if (account.alias !== store.account || account.server !== store.server)
    return null;
  const server = snapshot.servers.find(
    (candidate) => candidate.id === store.server,
  );
  if (!server) return null;
  return {
    account,
    store,
    server,
    available: storeReadable(snapshot, store.id),
  };
}

/** One accessible name per button, since several read "Check for groups". */
export const checkLabel = (context: DiscoveryContext): string =>
  `Check for groups accessible to ${context.account.username} on ${context.server.name}`;

export const unavailableTitle = (context: DiscoveryContext): string =>
  `Restore access to ${context.server.name} before checking for groups.`;

/**
 * Why an invitation cannot be written: the message names the server the
 * invitee joins, so it cannot be composed while that server is out of reach.
 */
export const inviteUnavailableTitle = (serverName: string): string =>
  `Restore access to ${serverName} before inviting someone.`;

/**
 * What a member row's second line says: the kind of party, and nothing else.
 * Only people and machines are drawn as member rows; a party that stands for
 * another group belongs to "Groups on other servers".
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
    return 'Memberships can’t be changed in an ad-hoc group.';
  if (!manageable)
    return 'Only an Admin or an Owner can change this group’s members.';
  if (party.label === 'you')
    return 'You cannot change your own role or remove your own account.';
  if (party.party_kind !== 'user' || !party.locally_manageable)
    return 'Members of an admitted group are managed on their own server and cannot be changed or removed one by one.';
  const mine = partiesOf(snapshot, party.store).find(
    (candidate) => candidate.label === 'you',
  );
  if (
    mine &&
    roleRank(party.destination_role) >= roleRank(mine.destination_role)
  )
    return 'An Admin cannot change another Admin or the Owner.';
  return 'This member cannot be changed from this Mac.';
}

/**
 * Why this Mac cannot change a group's roster, or the groups admitted into it —
 * `undefined` when it can. One rule for both the Teams list and the group page,
 * so a row's menu and the page it opens never disagree about what applies.
 */
export function manageReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
  source: 'roster' | 'federation',
): string | undefined {
  if (store.team_kind !== 'named')
    return 'Memberships can’t be changed in an ad-hoc group.';
  if (store.active === false) return 'Finish setting up this group first.';
  if (!storeReadable(snapshot, store.id))
    return `Restore access to ${serverOf(snapshot, store.id)?.name ?? store.server} first.`;
  if (groupDetailFailure(snapshot, store.id, source))
    return source === 'roster'
      ? 'The roster could not be read. Refresh before making changes.'
      : 'The admitted groups could not be read. Refresh before making changes.';
  // The role this Mac holds is a roster fact, so an unread roster is not
  // evidence that it lacks one: an admission is refused for what failed to
  // load, not for a permission nothing could have checked.
  if (groupDetailFailure(snapshot, store.id, 'roster'))
    return 'The roster could not be read. Refresh before making changes.';
  const mine = partiesOf(snapshot, store.id).find(
    (party) => party.label === 'you',
  );
  return mine && roleRank(mine.destination_role) >= 2
    ? undefined
    : 'Only an Admin or an Owner can change this group’s members.';
}

/** Whether this Mac can add to or change the group's roster. */
export function rosterManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'roster') === undefined;
}

/** Whether this Mac can admit another group here, or drop one. */
export function federationManageable(
  snapshot: AgentSnapshot,
  store: TeamStore,
): boolean {
  return manageReason(snapshot, store, 'federation') === undefined;
}

/**
 * Why leaving is unavailable: there is no leave command to offer. Which of the
 * two sentences applies can only be told from a roster that loaded — an empty
 * or unread roster is not evidence of sole ownership.
 */
export function leaveReason(snapshot: AgentSnapshot, store: Store): string {
  const roster = partiesOf(snapshot, store.id);
  if (!roster.length || groupDetailFailure(snapshot, store.id, 'roster'))
    return 'Leaving a group is not available yet.';
  const seniors = roster
    .filter(
      (party) =>
        party.party_kind === 'user' &&
        party.label !== 'you' &&
        roleRank(party.destination_role) >= 2,
    )
    .map((party) => partyName(party));
  return seniors.length
    ? `To leave this group, ask ${oxfordOr(seniors)} to remove your account.`
    : 'As the sole owner, you must transfer ownership or delete the group to leave.';
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
                title="Roles can only be lowered. To raise one, remove the member and add them again, which rotates the group key."
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
                title="Removes this member and rotates the group key."
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

function SituationBand({
  store,
  tab,
  onFinish,
}: {
  store: Store;
  tab: Tab;
  onFinish: () => void;
}): ReactNode {
  if (store.kind === 'team' && store.active === false) {
    return (
      <Band
        label="Setup incomplete."
        action={
          <Button variant="primary" size="sm" onClick={onFinish}>
            Finish setup
          </Button>
        }
      >
        Group members and items are unavailable until setup is finished.
      </Band>
    );
  }
  if (
    store.kind === 'team' &&
    store.team_kind === 'adhoc' &&
    tab !== 'settings'
  ) {
    return (
      <Band severity="info" label="Ad-hoc group">
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
        <span className="kico round" style={{ background: hue(name) }}>
          {name.slice(0, 1).toUpperCase()}
        </span>
        <span className="t">
          <b>
            <span>{name}</span>
          </b>
          <small>on {teamPartyHost(party)}</small>
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
 * The groups admitted from other servers. They keep their own facts — the
 * admission state, the host id and the operation id — because restoring access
 * needs the operation id.
 */
function FederationRows({
  snapshot,
  store,
  onRerun,
  onRemove,
  manageable,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  onRerun: (operationId: string) => void;
  onRemove: (entry: FederationEntry) => void;
  manageable: boolean;
}): ReactNode {
  const { entries, unmatched, ambiguous } = admittedGroups(snapshot, store);
  if (!entries.length && !unmatched.length && !ambiguous.length)
    return (
      <div className="callout">
        <span
          className="kico"
          style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
        >
          <Icon name="people" />
        </span>
        <span className="t">
          <b>No groups from other servers.</b>
        </span>
      </div>
    );
  return (
    <div className="rt bare fed">
      {entries.map((entry) => {
        const remoteName =
          snapshot.servers.find((server) => server.id === entry.remote_profile)
            ?.name ?? entry.remote_profile;
        const memberReason =
          'Every member of an admitted group holds the role shown on its row. They are managed on their own server and cannot be changed or removed one by one.';
        return (
          <div
            className="prow"
            key={`${entry.remote_host_id_hex}|${entry.remote_team_id_hex}`}
          >
            <span className="who2">
              <span
                className="kico round"
                style={{ background: hue(entry.remote_team_alias) }}
              >
                {entry.remote_team_alias.slice(0, 1).toUpperCase()}
              </span>
              <span className="t">
                <b>
                  <span>{entry.remote_team_alias}</span>
                </b>
                <small>
                  on {remoteName} · host <code>{entry.remote_host_id_hex}</code>
                  {entry.operation_id_hex ? (
                    <>
                      {' '}
                      · operation <code>{entry.operation_id_hex}</code>
                    </>
                  ) : null}
                </small>
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
                    <hr />
                    <MenuItem
                      danger
                      reason={
                        !manageable
                          ? 'Only an Admin or an Owner can remove this admission.'
                          : entry.active
                            ? undefined
                            : 'Restore access before removing this admission.'
                      }
                      title="Removes the whole admission and rotates this group’s key."
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
      })}
      {unmatched.map((party) => (
        <TeamPartyRow
          key={party.party_id_hex}
          party={party}
          chip="No admission record"
          chipTitle="The roster lists this group as a member, but no admission record on this Mac matches it."
        />
      ))}
      {ambiguous.map((party) => (
        <TeamPartyRow
          key={party.party_id_hex}
          party={party}
          chip="Ambiguous admission"
          chipTitle="The roster lists this group once, but several admission records on this Mac match it, so none of them can be acted on."
        />
      ))}
    </div>
  );
}

/**
 * The Members tab: the roster split into the people, the machines and the
 * groups admitted from other servers, then the actions that add to it.
 */
function MembersTab({
  snapshot,
  store,
  onSheet,
  onFinish,
  manageable,
  federationManageable,
  menuParty,
  failure,
  federationFailure,
  onRetry,
  onRetryFederation,
  onRerun,
  onRemoveAdmission,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onFinish: () => void;
  manageable: boolean;
  federationManageable: boolean;
  menuParty: string | null;
  failure?: GroupDetailFailure;
  federationFailure?: GroupDetailFailure;
  onRetry: () => void;
  onRetryFederation: () => void;
  onRerun: (operationId: string) => void;
  onRemoveAdmission: (entry: FederationEntry) => void;
}): ReactNode {
  const parties = sortRoster(partiesOf(snapshot, store.id));
  const people = parties.filter(
    (party) => party.party_kind === 'user' && !isMachine(party),
  );
  const machines = parties.filter((party) => isMachine(party));
  const inactive = store.kind === 'team' && store.active === false;
  const named = store.kind === 'team' && store.team_kind === 'named';
  const server = serverOf(snapshot, store.id);
  const serverName = server?.name ?? store.server;
  const readable = storeReadable(snapshot, store.id);
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
  const unavailableHint = failure
    ? 'The roster could not be read. Refresh before making changes.'
    : 'Not available for this group';
  return (
    <div className="roster">
      <SituationBand store={store} tab="people" onFinish={onFinish} />
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
      ) : inactive ? null : (
        <>
          <MemberSection title="People" count={peopleLabel(people.length)}>
            {people.length ? (
              people.map(row)
            ) : (
              <div className="callout">
                <span
                  className="kico"
                  style={{
                    background: 'var(--chip-bg)',
                    color: 'var(--muted)',
                  }}
                >
                  <Icon name="people" />
                </span>
                <span className="t">
                  <b>No people yet.</b>
                </span>
              </div>
            )}
          </MemberSection>
          {machines.length ? (
            <MemberSection
              title="Machines"
              count={`${machines.length} connected`}
            >
              {machines.map(row)}
            </MemberSection>
          ) : null}
          {/* The people and machine actions follow the rows they add to, so a
              roster that failed to load offers neither: there are no rows to
              add to, and the invitation names a server this Mac cannot read. */}
          {named ? (
            <div className="roster-actions">
              <Button
                variant="primary"
                icon="plus"
                disabled={!manageable}
                title={
                  manageable
                    ? 'Add someone who already has an account on this server'
                    : unavailableHint
                }
                onClick={() => onSheet('add')}
              >
                Add someone on {serverName}…
              </Button>
              <Button
                disabled={!readable}
                title={
                  readable ? undefined : inviteUnavailableTitle(serverName)
                }
                onClick={() => onSheet('invite')}
              >
                Invite someone…
              </Button>
            </div>
          ) : null}
        </>
      )}
      {inactive ? null : (
        <>
          <SectionLabel>Groups on other servers</SectionLabel>
          {federationFailure ? (
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
          ) : (
            <FederationRows
              snapshot={snapshot}
              store={store}
              onRerun={onRerun}
              onRemove={onRemoveAdmission}
              manageable={federationManageable}
            />
          )}
          {/* And the admission action follows the admissions. */}
          {named ? (
            <div className="roster-actions">
              <Button
                icon="people"
                disabled={!federationManageable}
                title={
                  federationManageable
                    ? 'Give every member of another group a role here'
                    : unavailableHint
                }
                onClick={() => onSheet('admit')}
              >
                Add a group…
              </Button>
            </div>
          ) : null}
        </>
      )}
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
  const apply = async (): Promise<void> => {
    if (!confirmed || busy || !entry.active) return;
    setBusy(true);
    try {
      await bridge.removeFederatedGroup({
        storeId: store.id,
        remoteHostIdHex: entry.remote_host_id_hex,
        remoteTeamIdHex: entry.remote_team_id_hex,
      });
      await onApplied(
        `${entry.remote_team_alias} removed and group keys rotated`,
      );
      onClose();
    } catch (error) {
      await onMutationError(error);
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
      subtitle="This targets the exact remote group and server shown below"
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
        this group. Group encryption keys will be updated to block future
        access.
      </p>
      <Inset>
        <InsetRow label="Remote host">
          <code>{entry.remote_host_id_hex}</code>
        </InsetRow>
        <InsetRow label="Remote group">
          <code>{entry.remote_team_id_hex}</code>
        </InsetRow>
      </Inset>
      <label className="checkline">
        <input
          type="checkbox"
          checked={confirmed}
          disabled={busy}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        I understand this removes every member of the remote group and rotates
        affected keys.
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
  onFinish,
  manageable,
  rekeyOpen,
}: {
  snapshot: AgentSnapshot;
  store: Extract<Store, { kind: 'team' }>;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onFinish: () => void;
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
      <SituationBand store={store} tab="settings" onFinish={onFinish} />
      <SectionLabel>About this group</SectionLabel>
      <Inset>
        <InsetRow label="Name">
          <span>
            {store.name}
            <span className="hint">A name is fixed at creation.</span>
          </span>
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
          {server?.name}
        </InsetRow>
        <InsetRow label="Your account">
          {account?.username ?? store.account} · {mine ? roleText(mine) : '—'}
        </InsetRow>
        <InsetRow label="Owner">
          {owner ? partyName(owner) : 'No owner designated'}
        </InsetRow>
        <InsetRow
          label="Group ID"
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
          <code>{store.team_id_hex}</code>
        </InsetRow>
      </Inset>
      {store.team_kind === 'adhoc' ? (
        <p className="fn">
          An ad-hoc group has no name on the server and a fixed membership.
        </p>
      ) : (
        <>
          <SectionLabel>Who can join</SectionLabel>
          <Inset>
            <InsetRow
              label="Join policy"
              action={
                // Inert rather than natively disabled, like a menu item that
                // does not apply: the keyboard still reaches it and reads why.
                <Button
                  size="sm"
                  aria-disabled
                  title={JOIN_POLICY_REASON}
                  onClick={undefined}
                >
                  Change…
                </Button>
              }
            >
              {/* The reason is read, not hovered: it is the row's own line. */}
              <span>
                Invite only
                <span className="hint">{JOIN_POLICY_REASON}</span>
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
            <b>Remove a group member</b>
            <small>
              Removing anyone rotates the group key and blocks their future
              reads. Copies already downloaded are not erased.
            </small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button
              variant="danger"
              disabled
              title={leaveReason(snapshot, store)}
            >
              Leave…
            </Button>
          }
        >
          <span className="t">
            <b>Leave {store.name}</b>
            <small>{leaveReason(snapshot, store)}</small>
          </span>
        </InsetRow>
        <InsetRow
          action={
            <Button variant="danger" disabled>
              Delete…
            </Button>
          }
        >
          <span className="t">
            <b>Delete {store.name}</b>
            <small>
              Permanently deletes the group and all shared items for all
              members.
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
            <b>Reset this Mac’s state for {server?.name}</b>
            <small>
              Removes locally stored keys and server data from this Mac.
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

export function GroupSheet({
  snapshot,
  bridge,
  store,
  sheet,
  target,
  onClose,
  onSwitch,
  onApplied,
  onError,
  onMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: Store;
  sheet: Exclude<Sheet, null>;
  target: Party | null;
  onClose: () => void;
  onSwitch: (sheet: Sheet, target?: Party) => void;
  onApplied: (
    message: string,
    created?: { accountStoreId: StoreRef; teamAlias: string },
  ) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const copy = useCopyText(bridge, onError);
  const toasts = useToast();
  const [username, setUsername] = useState('jules.park');
  const [visibility, setVisibility] = useState(0);
  const callerParty = partiesOf(snapshot, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const callerRank = callerParty ? roleRank(callerParty.destination_role) : 0;
  const [role, setRole] = useState<RoleDto>({
    role: 'Member',
    visibility: 0,
  });
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState('Platform');
  const [createKind, setCreateKind] = useState<'named' | 'adhoc'>('named');
  const creationAccounts = snapshot.stores.filter(
    (candidate): candidate is AccountStore =>
      candidate.kind === 'account' && canCreateInStore(snapshot, candidate.id),
  );
  // Creating acts as the account the page acts as: the sheet's own store when
  // that store is an account that can create, else this Mac's first such one.
  const [accountStoreId, setAccountStoreId] = useState(
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
      storeReadable(snapshot, candidate.id) &&
      candidate.server !== store.server,
  );
  const [remoteStoreId, setRemoteStoreId] = useState(remotes[0]?.id ?? '');
  const creationAccount = creationAccounts.find(
    (candidate) => candidate.id === accountStoreId,
  );
  const server = serverOf(
    snapshot,
    sheet === 'create' && creationAccount ? creationAccount.id : store.id,
  );
  const ownerStore =
    store.kind === 'team'
      ? snapshot.stores.find(
          (candidate) =>
            candidate.kind === 'account' &&
            candidate.server === store.server &&
            candidate.account === store.account,
        )
      : undefined;
  const ownerAccount = ownerStore
    ? snapshot.accounts.find(
        (account) =>
          account.store === ownerStore.id ||
          (account.server === ownerStore.server &&
            account.alias === ownerStore.account),
      )
    : snapshot.accounts.find((account) => account.store === store.id);
  // The username as the server holds it: the whole of it is what the reader
  // types back, so it is not abbreviated here.
  const inviter = ownerAccount?.username ?? 'the group Admin';
  const serverName = server?.name ?? store.server;
  // The same command, aimed at whichever store opened the sheet: a group
  // invites someone into that group, an account invites them onto its server.
  const inviteMessage =
    store.kind === 'team'
      ? `I'd like to invite you to join ${store.name} on FOKS.\n\n1. Download FOKS: https://foks.app/download\n2. Add server: ${serverName}\n3. Create your account on the server\n4. Send your username to ${inviter}\n\nOnce added, ${store.name} will appear in your Teams list.`
      : `I'd like to invite you to FOKS on ${serverName}.\n\n1. Download FOKS: https://foks.app/download\n2. Add server: ${serverName}\n3. Create your account on the server\n4. Send your username to ${inviter}\n\nOnce I add your username to a group, it will appear in your Teams list.`;
  const teamAlias = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
  const remote =
    remotes.find((candidate) => candidate.id === remoteStoreId) ?? remotes[0];
  const requiredFailure =
    sheet === 'admit'
      ? groupDetailFailure(snapshot, store.id, 'federation')
      : ['add', 'demote', 'remove'].includes(sheet)
        ? groupDetailFailure(snapshot, store.id, 'roster')
        : undefined;
  const title =
    sheet === 'invite'
      ? `Invite someone to ${store.kind === 'team' ? store.name : serverName}`
      : sheet === 'add'
        ? `Add someone to ${store.name}`
        : sheet === 'demote'
          ? `Lower ${target ? `${partyName(target)}’s` : 'their'} role`
          : sheet === 'remove'
            ? `Remove ${target ? partyName(target) : 'them'} from ${store.name}?`
            : sheet === 'admit'
              ? `Add a group to ${store.name}`
              : 'Create a group';
  const subtitle =
    sheet === 'create'
      ? // Creating acts as one account on one server, and the sheet says which:
        // the server's name alone would not say who is creating the group.
        creationAccount
        ? accountSubtitle(snapshot, creationAccount)
        : 'No account on this Mac can create a group'
      : sheet === 'invite'
        ? 'Send instructions to help a new user set up their account'
        : sheet === 'add'
          ? 'Add an existing user from this server'
          : sheet === 'demote'
            ? `${target ? roleText(target) : ''} in ${store.name} today`
            : sheet === 'remove'
              ? ''
              : sheet === 'admit'
                ? 'Every member of that group gets the same role here'
                : server?.name;
  const [demotion, setDemotion] = useState<RoleDto | null>(() =>
    target ? demotionFor(target) : null,
  );
  const currentRole = target ? parseRole(target.destination_role) : null;
  const maxMemberVisibility =
    currentRole?.kind === 'member' ? (currentRole.visibility ?? 0) - 1 : 0;
  useEffect(() => {
    if (sheet === 'demote') setDemotion(target ? demotionFor(target) : null);
  }, [sheet, target]);
  const apply = async (): Promise<void> => {
    if (busy || requiredFailure) return;
    setBusy(true);
    try {
      if (sheet === 'invite') {
        await bridge.copyText(inviteMessage);
        toasts.show('Message copied.');
      } else if (sheet === 'add')
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
      else if (sheet === 'admit' && remote)
        await bridge.admitGroup({
          storeId: store.id,
          remoteStoreId: remote.id,
          visibility,
        });
      else if (sheet === 'create') {
        // Re-resolved from the availability-filtered list at the moment of
        // the write, not from the render that drew the button: the server
        // can lapse while the sheet is open.
        const account = creationAccounts.find(
          (candidate) => candidate.id === accountStoreId,
        );
        if (!account)
          throw new Error('No account store is available for group creation.');
        await bridge.createGroup({
          accountStoreId: account.id,
          teamAlias,
          name: createKind === 'named' ? name : '',
          kind: createKind,
        });
        await onApplied(`${title} completed`, {
          accountStoreId: account.id,
          teamAlias,
        });
        onClose();
        return;
      }
      if (sheet !== 'invite') await onApplied(`${title} completed`);
      onClose();
    } catch (error) {
      if (sheet === 'invite') onError(error);
      else await onMutationError(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <SheetDialog
      danger={sheet === 'remove'}
      onClose={onClose}
      dismissible={!busy}
      width={sheet === 'invite' ? 'wide' : 'base'}
      glyph={
        sheet === 'create' ? (
          // The group does not exist yet, so it has no mark: a group's colour
          // and initial are earned at creation, not previewed over an account.
          <span className="kico md neutral">
            <Icon name="people" />
          </span>
        ) : store.kind === 'team' ? (
          <GroupMark store={store} />
        ) : (
          // An account store opens the invite sheet for its server, not a group.
          <span className="server-mark">
            <Icon name="server" />
          </span>
        )
      }
      title={title}
      subtitle={subtitle}
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
              disabled={
                busy ||
                Boolean(requiredFailure) ||
                (sheet === 'add' && !username.trim()) ||
                (sheet === 'demote' &&
                  (!target || !canTarget(snapshot, target) || !demotion)) ||
                (sheet === 'admit' && !remote) ||
                (sheet === 'create' && (!teamAlias || !creationAccount))
              }
              onClick={() => void apply()}
            >
              {sheet === 'invite'
                ? 'Copy message'
                : sheet === 'add'
                  ? `Add ${username.trim() || 'someone'}`
                  : sheet === 'demote'
                    ? 'Change role'
                    : sheet === 'admit'
                      ? 'Add group'
                      : // The alias is what the server is asked to create.
                        `Create ${teamAlias || 'group'}`}
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
        {sheet === 'invite' ? (
          <>
            <p>
              They need an account on {serverName} before you can add them. Send
              this message, then add their username.
            </p>
            <Inset>
              <InsetRow label="Message">
                <span className="msg">{inviteMessage}</span>
                <Button
                  size="sm"
                  onClick={() => void copy(inviteMessage, 'Message copied.')}
                >
                  Copy
                </Button>
              </InsetRow>
            </Inset>
            <SectionLabel>Message contents</SectionLabel>
            <Inset className="rows">
              <InsetRow label="Install link">
                <code>https://foks.app/download</code>{' '}
                <Chip tone="warn">placeholder</Chip>
                <span className="hint">Temporary download address.</span>
              </InsetRow>
              <InsetRow label="Server">
                <span>
                  {server?.name}
                  <span className="hint">
                    They enter this address during setup.
                  </span>
                </span>
              </InsetRow>
              <InsetRow label="Signup invite">
                <span>
                  Optional
                  <span className="hint">
                    Required only if the server requires an invite.
                  </span>
                </span>
              </InsetRow>
              <InsetRow label="When they reply">
                <span>Add their username as a Member, Admin, or Owner.</span>
                {store.kind === 'team' ? (
                  <Button size="sm" onClick={() => onSwitch('add')}>
                    Add someone
                  </Button>
                ) : null}
              </InsetRow>
            </Inset>
            <p className="hint">
              Send this message to the person you want to invite.
            </p>
          </>
        ) : null}
        {sheet === 'add' ? (
          <>
            <Inset>
              <Field label="Username" value={username} onChange={setUsername} />
            </Inset>
            <SectionLabel>Role in {store.name}</SectionLabel>
            <Inset>
              <RadioGroup label={`Role in ${store.name}`}>
                {(callerRank >= 3
                  ? (['Member', 'Admin', 'Owner'] as const)
                  : (['Member', 'Admin'] as const)
                ).map((next) => (
                  <RadioCard
                    key={next}
                    selected={role.role === next}
                    onSelect={() =>
                      setRole(
                        next === 'Member'
                          ? { role: next, visibility }
                          : { role: next },
                      )
                    }
                    title={next}
                    detail={
                      next === 'Member'
                        ? 'Opens items at or above its visibility band. Visibility 0 is the default.'
                        : next === 'Admin'
                          ? 'Changes items and adds or removes people. Cannot change other Admins or the Owner.'
                          : 'Everything, including deleting the group.'
                    }
                  />
                ))}
              </RadioGroup>
            </Inset>
            {role.role === 'Member' ? (
              <div className="vis">
                <Button
                  size="sm"
                  disabled={visibility <= VIS_MIN}
                  onClick={() => setVisibility((value) => value - 1)}
                >
                  −
                </Button>
                <span>Visibility {visibility}</span>
                <Button
                  size="sm"
                  disabled={visibility >= VIS_MAX}
                  onClick={() => setVisibility((value) => value + 1)}
                >
                  +
                </Button>
              </div>
            ) : null}
            <p className="fn">
              They can access items allowed by their role immediately. No
              invitation is sent; {store.name} will appear when their app checks
              the server.
            </p>
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
                    detail="Full access to group items and permission to manage members."
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
              Removing blocks future reads and rekeys the group. This user may
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
            <SectionLabel>Group</SectionLabel>
            <Inset>
              {remotes.length ? (
                <RadioGroup label="Group">
                  {remotes.map((group) => {
                    const host = serverOf(snapshot, group.id);
                    return (
                      <RadioCard
                        key={group.id}
                        selected={remote?.id === group.id}
                        onSelect={() => setRemoteStoreId(group.id)}
                        title={group.alias}
                        detail={`on ${host?.name ?? group.server}`}
                      />
                    );
                  })}
                </RadioGroup>
              ) : (
                <InsetRow label="Group">
                  <span className="dim">No eligible remote group</span>
                </InsetRow>
              )}
            </Inset>
            <p className="fn">
              You can add groups that are already visible from this device.
            </p>
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
                      disabled={visibility <= VIS_MIN}
                      onClick={() => setVisibility((value) => value - 1)}
                    >
                      −
                    </Button>
                    <span className="vis-value">Visibility {visibility}</span>
                    <Button
                      size="sm"
                      disabled={visibility >= VIS_MAX}
                      onClick={() => setVisibility((value) => value + 1)}
                    >
                      +
                    </Button>
                  </>
                }
              />
            </Inset>
            <Band severity="info">
              {store.name} automatically syncs members from that group.
            </Band>
          </>
        ) : null}
        {sheet === 'create' ? (
          <>
            <Inset>
              <Field label="Name" value={name} onChange={setName} />
            </Inset>
            <p className="fn">
              Others find it as <code>{teamAlias || '…'}</code> on the server. A
              name is fixed at creation.
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
                      title={serverOf(snapshot, account.id)?.name}
                      detail={`as ${snapshot.accounts.find((candidate) => candidate.store === account.id || (candidate.alias === account.account && candidate.server === account.server))?.username ?? account.account}`}
                    />
                  ))}
                </RadioGroup>
              ) : (
                <InsetRow label="Account">
                  <span className="dim">No account can create a group.</span>
                </InsetRow>
              )}
            </Inset>
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
  onNavigate,
  onApplied: onSnapshotApplied,
  onError,
  onMutationError: onSnapshotMutationError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'group-settings' }>;
  onNavigate: (location: Location) => void;
  onApplied: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}): ReactNode {
  const copy = useCopyText(bridge, onError);
  const initial = stateName();
  const [tab, setTab] = useState<Tab>(
    () => location.tab ?? tabFromState(initial),
  );
  const [sheet, setSheet] = useState<Sheet>(() =>
    ['invite', 'add', 'demote', 'remove', 'admit', 'party-remove'].includes(
      initial,
    )
      ? initial === 'party-remove'
        ? 'remove'
        : (initial as Sheet)
      : null,
  );
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
  // An interrupted member addition or role change leaves durable local state
  // that blocks every later membership mutation until it is resumed. Read the
  // account's pending operations for this group so the UI can finish it.
  const [membershipPending, setMembershipPending] = useState<
    PendingOperation[]
  >([]);
  const pendingProfile = store?.kind === 'team' ? store.server : undefined;
  const pendingAlias = store?.kind === 'team' ? store.alias : undefined;
  const loadMembershipPending = useCallback(async (): Promise<void> => {
    if (!pendingProfile || !pendingAlias) {
      setMembershipPending([]);
      return;
    }
    try {
      const rows = await enqueueProfileWork(bridge, pendingProfile, () =>
        bridge.listPendingOperations(pendingProfile),
      );
      setMembershipPending(
        rows.filter(
          (row) =>
            row.alias === pendingAlias &&
            (row.kind === 'team-member-addition' ||
              row.kind === 'team-member-edit'),
        ),
      );
    } catch {
      setMembershipPending([]);
    }
  }, [bridge, pendingAlias, pendingProfile]);
  useEffect(() => {
    void loadMembershipPending();
  }, [loadMembershipPending]);
  // Both successful changes and reconciled failures can change the durable
  // pending records. Refresh them for every membership action and manual refresh.
  const onApplied = useCallback(
    async (message: string): Promise<void> => {
      try {
        await onSnapshotApplied(message);
      } finally {
        await loadMembershipPending();
      }
    },
    [loadMembershipPending, onSnapshotApplied],
  );
  const onMutationError = useCallback<MutationFailureHandler>(
    async (error, options) => {
      try {
        await onSnapshotMutationError(error, options);
      } finally {
        await loadMembershipPending();
      }
    },
    [loadMembershipPending, onSnapshotMutationError],
  );
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
    if (seenStore.current && storeId && seenStore.current !== storeId) {
      setTab('people');
      setSheet(null);
      setTarget(null);
      setRemovalTarget(null);
      setRekeyArmed(false);
    }
    if (seenStore.current && !storeId) {
      setTab('people');
      setSheet(null);
      setTarget(null);
      setRemovalTarget(null);
      setRekeyArmed(false);
    }
    seenStore.current = storeId;
  }, [storeId]);
  const mutate = async (
    action: () => Promise<unknown>,
    message: string,
  ): Promise<void> => {
    try {
      await action();
      await onApplied(message);
    } catch (error) {
      await onMutationError(error);
    }
  };
  const resumeMembership = (operation: PendingOperation): void => {
    if (!store || store.kind !== 'team') return;
    if (operation.kind === 'team-member-addition') {
      const username = operation.target;
      if (!username) return;
      void mutate(
        () => bridge.resumeGroupMemberAddition({ storeId: store.id, username }),
        'Member addition resumed',
      );
      return;
    }
    if (operation.kind === 'team-member-edit') {
      void mutate(
        () => bridge.resumeGroupMemberEdit(store.id),
        'Member change resumed',
      );
    }
  };

  if (!store || store.kind !== 'team') {
    return (
      <>
        <PageHeader title="Group unavailable" subtitle="" />
        <div className="body">
          <Notice title="This group is no longer available">
            Refresh the catalog or choose another group from Teams.
          </Notice>
        </div>
      </>
    );
  }
  const access = storeDescriptionState(snapshot, store);
  const unavailable = access !== 'normal' && access !== 'setup-incomplete';
  const inactive = store.active === false;
  const callerParty = partiesOf(snapshot, store.id).find(
    (candidate) => candidate.label === 'you',
  );
  const canManageRoster = rosterManageable(snapshot, store);
  const canManageFederation = federationManageable(snapshot, store);
  const finishSetup = (): void => {
    void mutate(
      () => bridge.resumeGroupCreation(store.id),
      'Group creation resumed',
    );
  };
  return (
    <>
      <div className="ghero">
        <button
          type="button"
          className="back"
          title="Back to Teams"
          aria-label="Back to Teams"
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
          {/* The server, then the bare role this account holds here. */}
          <div className="sub">
            {[
              serverOf(snapshot, store.id)?.name ?? store.server,
              callerParty
                ? (() => {
                    const parsed = parseRole(callerParty.destination_role);
                    return parsed ? roleName(parsed) : null;
                  })()
                : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          </div>
        </div>
        <div className="header-action">
          {/* The group ID and the vault are local facts, so the menu stays even
              while the server is out of reach; only what needs the server is
              disabled, with the reason in the item. */}
          <MenuButton
            variant="quiet"
            icon="more"
            trailingIcon={null}
            label=""
            menuLabel="Group actions"
            // Named for the group it acts on: several triggers on this page
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
                      ? `Restore access to ${serverOf(snapshot, store.id)?.name ?? store.server} first.`
                      : inactive
                        ? 'Finish setting up this group first.'
                        : undefined
                  }
                  onClick={() => {
                    close();
                    void mutate(() => Promise.resolve(), 'Group refreshed');
                  }}
                >
                  Refresh group
                </MenuItem>
                <MenuItem
                  icon="copy"
                  onClick={() => {
                    close();
                    void copy(store.team_id_hex, 'Group ID copied.');
                  }}
                >
                  Copy group ID
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
          snapshot={snapshot}
          store={store}
          noHeader
          variant="band"
          onOpenServer={(profile) =>
            onNavigate({ kind: 'settings', section: 'servers', profile })
          }
          onFinishSetup={finishSetup}
        />
      ) : (
        <>
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
            label="Group sections"
            idBase={GROUP_TABS}
            value={tab}
            onChange={(next) => {
              setTab(next);
              onNavigate({ kind: 'group-settings', ref: store.id, tab: next });
            }}
            items={[
              {
                id: 'people',
                label: 'Members',
                ...(rosterFailure || federationFailure
                  ? {}
                  : { count: memberCount }),
              },
              { id: 'settings', label: 'Settings' },
            ]}
          />
          {/* Fixed IDs associate each tab button with its tabpanel. */}
          <div
            className="body"
            role="tabpanel"
            id={tabPanelId(GROUP_TABS, tab)}
            aria-labelledby={tabId(GROUP_TABS, tab)}
          >
            <div className="groups-wrap">
              {tab === 'people' ? (
                <MembersTab
                  snapshot={snapshot}
                  store={store}
                  onSheet={openSheet}
                  onFinish={finishSetup}
                  manageable={canManageRoster}
                  federationManageable={canManageFederation}
                  menuParty={menuParty}
                  failure={rosterFailure}
                  federationFailure={federationFailure}
                  onRetry={() => void onApplied('Refreshing group members…')}
                  onRetryFederation={() =>
                    void onApplied('Refreshing external groups…')
                  }
                  onRerun={(operationId) =>
                    void mutate(
                      () => bridge.rerunGroupAdmission(store.id, operationId),
                      'Group access restored',
                    )
                  }
                  onRemoveAdmission={setRemovalTarget}
                />
              ) : (
                <SettingsTab
                  snapshot={snapshot}
                  store={store}
                  onSheet={openSheet}
                  onNavigate={onNavigate}
                  onCopy={(text) => void copy(text, 'Group ID copied.')}
                  onFinish={finishSetup}
                  manageable={canManageRoster}
                  rekeyOpen={rekeyArmed}
                />
              )}
              {tab === 'settings' || (!rosterFailure && !federationFailure) ? (
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
              snapshot={snapshot}
              bridge={bridge}
              store={store}
              sheet={sheet}
              target={targetParty}
              onClose={() => setSheet(null)}
              onSwitch={openSheet}
              onApplied={onApplied}
              onError={onError}
              onMutationError={onMutationError}
            />
          ) : null}
        </>
      )}
    </>
  );
}
