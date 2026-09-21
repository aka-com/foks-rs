/**
 * The channel info panel.
 *
 * What the model already holds about the open channel: its description, its
 * visibility, the team roster with FOKS roles, and the per-device alert
 * settings that used to sit in a strip above every thread. Leaving, muting,
 * deleting and editing the description have no `ChatAction` yet, so this panel
 * does not draw them.
 */

import type { ReactNode } from 'react';
import { Button } from '../components';
import { NotificationSettings } from '../chat/notification-provider';
import {
  groupDetailFailure,
  parseRole,
  partiesOf,
  partyName,
  roleName,
  roleRank,
} from '../model';
import type { AgentSnapshot, Party, TeamStore } from '../model';
import type { ChatChannel, ChatScope } from '../chat-contract';
import type { Location } from '../location';

/** "Owner", "Admin", "Member" — the shell's role name, never the band. */
function roleLabel(party: Party): string {
  const role = parseRole(party.destination_role);
  return role ? roleName(role) : 'Role unavailable';
}

export function ChannelInfoPanel({
  snapshot,
  store,
  channel,
  loading = false,
  scope,
  onNavigate,
  onClose,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  /** Undefined while the channel inbox is loading. */
  channel?: ChatChannel;
  /** True while awaiting the channel inbox response. */
  loading?: boolean;
  scope?: ChatScope;
  onNavigate: (location: Location) => void;
  onClose: () => void;
}): ReactNode {
  const rosterFailure = groupDetailFailure(snapshot, store.id, 'roster');
  const parties = [...partiesOf(snapshot, store.id)].sort(
    (left, right) =>
      roleRank(right.destination_role) - roleRank(left.destination_role),
  );
  return (
    <aside className="chat-info" aria-label="Channel info">
      <div className="chat-info-head">
        <b>Channel info</b>
        <Button
          variant="quiet"
          icon="x"
          aria-label="Close channel info"
          title="Close"
          onClick={onClose}
        />
      </div>
      <section>
        <h3>Description</h3>
        {/* A team switch keeps the panel open while the new team's inbox
            arrives: what is known about the team is drawn meanwhile. */}
        <p>
          {channel
            ? channel.description || 'No description.'
            : loading
              ? 'Loading the channel…'
              : 'No channel is open.'}
        </p>
      </section>
      {channel && (
        <section>
          <h3>Visibility</h3>
          <p>
            <b>
              {channel.admin ? 'Admins and owners' : 'Everyone on the team'}
            </b>
          </p>
          <p className="chat-quiet">
            {channel.admin
              ? 'Hidden from members. Only admins and owners can read or write.'
              : 'Members and administrators can view and post messages.'}
          </p>
        </section>
      )}
      <section>
        {/* The conversation header already carries the channel's access line. */}
        <h3>{rosterFailure ? 'Members' : `Members · ${parties.length}`}</h3>
        {rosterFailure ? (
          <p role="alert">{rosterFailure.message}</p>
        ) : (
          <div className="chat-info-members">
            {parties.map((party) => (
              <div className="chat-info-member" key={party.party_id_hex}>
                <span className="t">
                  {partyName(party)}
                  {party.label === 'you' && <small>You</small>}
                </span>
                <span className="chip">{roleLabel(party)}</span>
              </div>
            ))}
          </div>
        )}
        <Button
          icon="people"
          title="Manage membership and access to team files and chat"
          onClick={() =>
            onNavigate({
              kind: 'group-settings',
              ref: store.id,
              tab: 'people',
            })
          }
        >
          Manage in Teams
        </Button>
      </section>
      {channel && (
        <section>
          <h3>Alerts on this device</h3>
          <Button
            onClick={() =>
              onNavigate({ kind: 'settings', section: 'preferences' })
            }
          >
            Device notification settings
          </Button>
          <NotificationSettings
            storeId={store.id}
            scope={scope}
            channel={channel.id}
          />
        </section>
      )}
    </aside>
  );
}
