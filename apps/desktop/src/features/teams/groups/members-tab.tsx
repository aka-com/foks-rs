/**
 * The group page's Members tab: the people, machines and federated teams of
 * one group, each row with the actions that apply to it.
 */

import { groupDetailReadiness, combineReadiness } from '../../../model';
import { CollectionStatus } from '../../../components/collection-status';
import {
  federatedTeamMembers,
  memberCountOf,
} from '../../../model/team-members';
import { canTarget, demotionFor, partySubtitle } from './sheet-support';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  Icon,
  MenuButton,
  MenuItem,
  PartyRow,
  RoleChip,
  SectionLabel,
} from '../../../components';
import {
  hue,
  isMachine,
  parseRole,
  partiesOf,
  partyName,
  plural,
  serverDisplayLabel,
} from '../../../model';
import type {
  FederationEntry,
  GroupDetailFailure,
  Party,
  Store,
  AgentSnapshot,
} from '../../../model';
import { sortRoster, targetReason } from '../../../model/group-model';
import type { GroupSheetKind } from './group-sheet';
import { SituationBand } from './situation-band';

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
function MemberRow({
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
  onSheet: (sheet: GroupSheetKind | null, party?: Party) => void;
}): ReactNode {
  const actionable = manageable && canTarget(snapshot, party);
  const reason = actionable
    ? undefined
    : targetReason(snapshot, store, party, manageable);
  const name = partyName(party);
  const lowerable = Boolean(demotionFor(party));
  // Only people and machines reach this row, and their access is their
  // membership: there is no inactive state to dim.
  return (
    <PartyRow
      variant="roster"
      party={party}
      mark={<PartyMark party={party} />}
      caption={partySubtitle(party)}
      actions={
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
                  actionable &&
                  parseRole(party.destination_role)?.kind !== 'owner'
                    ? undefined
                    : (reason ?? 'This member already has the highest role.')
                }
                onClick={() => {
                  close();
                  onSheet('promote', party);
                }}
              >
                Raise role…
              </MenuItem>
              <MenuItem
                reason={
                  actionable
                    ? lowerable
                      ? undefined
                      : 'This member is already at the lowest role.'
                    : reason
                }
                title="Lowers access and rotates the affected team keys."
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
      }
    />
  );
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
    <PartyRow
      variant="roster"
      party={party}
      name={name}
      mark={
        <span
          className="kico round"
          style={{ background: hue(name) }}
          aria-hidden="true"
        >
          {name.slice(0, 1).toUpperCase()}
        </span>
      }
      caption={`team on ${teamPartyHost(party)}`}
      actions={
        <Chip tone="warn" title={chipTitle}>
          {chip}
        </Chip>
      }
    />
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
export function MembersTab({
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
  onSheet: (sheet: GroupSheetKind | null, party?: Party) => void;
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
  const memberReadiness = combineReadiness([
    groupDetailReadiness(snapshot, store.id, 'roster'),
    groupDetailReadiness(snapshot, store.id, 'federation'),
  ]);
  const parties = sortRoster(partiesOf(snapshot, store.id));
  const people = parties.filter(
    (party) => party.party_kind === 'user' && !isMachine(party),
  );
  const machines = parties.filter((party) => isMachine(party));
  const row = (party: Party): ReactNode => (
    <MemberRow
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
        {!failure && !federationFailure && memberReadiness === 'ready' ? (
          <span className="count">
            — {plural(memberCountOf(snapshot, store), 'member')}
          </span>
        ) : null}
      </SectionLabel>
      {!failure && !federationFailure ? (
        <CollectionStatus state={memberReadiness} label="members" />
      ) : null}
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
      ) : memberReadiness !== 'ready' ? null : (
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
