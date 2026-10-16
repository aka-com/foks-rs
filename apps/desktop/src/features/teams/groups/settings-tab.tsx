/**
 * The group page's Settings tab: the team's facts, its nesting order and the
 * danger zone.
 */

import { canTarget } from './sheet-support';
import type { ReactNode } from 'react';
import {
  Button,
  Inset,
  InsetRow,
  MenuButton,
  SectionLabel,
} from '../../../components';
import {
  catalog,
  parseRole,
  partiesOf,
  partyName,
  roleChipText,
  roleRank,
  serverDisplayLabel,
  serverOf,
  shortId,
  visibilityOf,
} from '../../../model';
import type { Item, Party, Store, AgentSnapshot } from '../../../model';
import type { Location } from '../../../location';
import { memberReads } from '../exposure-review-model';
import type { GroupSheetKind } from './group-sheet';
import { SituationBand } from './situation-band';

function itemsOf(snapshot: AgentSnapshot, store: string): Item[] {
  return catalog(snapshot).filter((item) => item.store === store);
}

export function SettingsTab({
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
  onSheet: (sheet: GroupSheetKind | null, party?: Party) => void;
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
          {account?.username ?? store.account} ·{' '}
          {mine ? roleChipText(mine.destination_role) : '—'}
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
                        {roleChipText(party.destination_role)} · reads{' '}
                        {memberReads(snapshot, party).length} of {total}
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
