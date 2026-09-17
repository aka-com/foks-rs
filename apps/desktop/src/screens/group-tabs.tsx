/**
 * The group's Channels and Files tabs, plus the page shown when group setup is
 * incomplete.
 *
 * Channels lists the inbox service's data for this group and opens each row in
 * Chat. Files links to the group's vault and displays the catalog item count.
 * Both tabs explain empty states.
 */

import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  Icon,
  Inset,
  InsetRow,
  SectionLabel,
} from '../components';
import {
  catalog,
  chatAvailable,
  plural,
  serverAvailability,
  serverDisplayName,
  serverName as displayServerName,
  serverOf,
  shortId,
  storeDescriptionState,
} from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  AvailabilityReason,
  TeamStore,
} from '../model';
import { useSidebarInbox } from '../chat/inbox-provider';
import {
  accessSummary,
  channelMeta,
  channelTitle,
  listChannels,
  previewTime,
} from '../chat/presentation';
import type { Location } from '../location';
import { accessCopy } from './store-access';

/** The items the catalog holds for one store. */
export function itemCountOf(snapshot: AgentSnapshot, store: TeamStore): number {
  return catalog(snapshot).filter((item) => item.store === store.id).length;
}

/**
 * Why this group has no channel list at all, or `undefined` when it does. A
 * server that does not offer chat is the one reason a group can never have
 * one; the rest are states that pass.
 */
function noChannelsReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
): string | undefined {
  if (store.team_kind !== 'named')
    return 'Chat is only available in teams, not shared folders.';
  const server = serverOf(snapshot, store.id);
  if (!server) return 'This team’s server is not configured on this device.';
  if (!server.capabilities.chat)
    return `Chat is not enabled on ${serverDisplayName(server)}.`;
  return undefined;
}

/**
 * Why chat cannot be reached for this group at the tab's own clock instant:
 * the store's condition where it has one, else its server's. Both are the
 * facts `StoreAccessTakeover` states, so the band says what the takeover
 * would have said had the page been drawn a moment later.
 */
function unreachableReason(
  snapshot: AgentSnapshot,
  store: TeamStore,
  options: AvailabilityOptions,
): AvailabilityReason | undefined {
  const state = storeDescriptionState(snapshot, store, options);
  if (state !== 'normal') return state;
  const server = serverOf(snapshot, store.id);
  if (!server) return 'vault-unavailable';
  const availability = serverAvailability(snapshot, server, options);
  return availability.available ? undefined : availability.reason;
}

/**
 * The group's chat channels, as the inbox service holds them: the name, what
 * the channel says about itself, who can take part, and its unread count.
 */
export function ChannelsTab({
  snapshot,
  store,
  accessOptions = {},
  onNavigate,
  onAddChannel,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  accessOptions?: AvailabilityOptions;
  onNavigate: (location: Location) => void;
  onAddChannel: () => void;
}): ReactNode {
  const inbox = useSidebarInbox();
  const entry = inbox.get(store.id);
  const offered = noChannelsReason(snapshot, store);
  const reachable = chatAvailable(snapshot, store, accessOptions);
  if (offered)
    return (
      <div className="roster">
        <Band severity="info" label="No channels here">
          {offered}
        </Band>
      </div>
    );
  if (!reachable) {
    // Why, read off the same access decision the takeover states, on the tab's
    // own clock: a lapse while this tab is open is a condition of the store or
    // of its server, never the roster summary a store's description falls back
    // to once nothing is wrong with it.
    const state = unreachableReason(snapshot, store, accessOptions);
    const serverName = displayServerName(snapshot, store);
    const copy = state ? accessCopy(state, store, serverName) : undefined;
    return (
      <div className="roster">
        <Band label={copy?.title ?? 'Channels unavailable'}>
          {copy?.detail ??
            `Channels in ${store.name} are unavailable until access to ${serverName} is restored.`}
        </Band>
      </div>
    );
  }
  const failed =
    entry !== undefined &&
    (entry.state === 'unavailable' ||
      entry.state === 'blocked' ||
      (Boolean(entry.error) && !entry.data));
  if (failed)
    return (
      <div className="roster">
        <Band label="Channels unavailable">
          {entry.error || 'Could not load this team’s channels.'}
        </Band>
      </div>
    );
  const listed = entry?.data
    ? listChannels(entry.data.channels, entry.data.conversations)
    : undefined;
  const now =
    accessOptions.nowSeconds === undefined
      ? Date.now()
      : accessOptions.nowSeconds * 1000;
  return (
    <div className="roster">
      {/* A synchronization that brought data and still could not finish
          everything is a note over the rows it did bring, the way the Chat
          column draws one, rather than a failure that replaces them. */}
      {entry?.data && entry.error ? (
        <Band severity="warn" label="Channels may be out of date">
          {entry.error}
        </Band>
      ) : null}
      <SectionLabel>
        Channels
        {listed === undefined ? null : (
          <span className="count">— {plural(listed.length, 'channel')}</span>
        )}
      </SectionLabel>
      {listed === undefined ? (
        <p className="fn" role="status">
          Loading channels…
        </p>
      ) : listed.length === 0 ? (
        <div className="callout">
          <span
            className="kico"
            style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
          >
            <Icon name="chat" />
          </span>
          <span className="t">
            <b>No channels yet.</b>
          </span>
        </div>
      ) : (
        <div className="rt bare">
          {listed.map((option) => {
            const meta = channelMeta(option, entry?.blockedChannels);
            const about =
              option.channel.description || accessSummary(option.channel);
            const unread =
              option.conversation && BigInt(option.conversation.unread) > 0n
                ? option.conversation.unread
                : null;
            // When the channel was last active, as the inbox row draws it: the
            // same stamp the Chat column reads, in the same words.
            const preview = option.conversation?.preview;
            const when = preview ? previewTime(preview.insert_time, now) : '';
            return (
              <div className="prow" key={option.channel.id}>
                <span className="who2">
                  <span
                    className="kico round"
                    style={{
                      background: 'var(--chip-bg)',
                      color: 'var(--muted)',
                    }}
                    aria-hidden="true"
                  >
                    <Icon name="chat" />
                  </span>
                  <span className="t">
                    <b>
                      <span>{channelTitle(option.channel)}</span>
                    </b>
                    <small>{meta ? `${meta} · ${about}` : about}</small>
                  </span>
                </span>
                <span className="rowtail">
                  {when ? <span className="when">{when}</span> : null}
                  {unread ? <Chip tone="warn">{unread} unread</Chip> : null}
                  <Button
                    size="sm"
                    aria-label={`Open ${channelTitle(option.channel)} in Chat`}
                    onClick={() =>
                      onNavigate({
                        kind: 'chat',
                        ref: store.id,
                        channel: option.channel.id,
                      })
                    }
                  >
                    Open in Chat
                  </Button>
                </span>
              </div>
            );
          })}
        </div>
      )}
      <div className="roster-actions">
        <Button
          icon="plus"
          disabled={listed === undefined}
          title={
            listed === undefined
              ? 'Wait for this team’s channels before adding one.'
              : 'Create a channel in this team'
          }
          onClick={onAddChannel}
        >
          Add channel
        </Button>
        <p className="fn">
          Channels are encrypted with the team key. Access is determined by each
          member’s assigned role.
        </p>
      </div>
    </div>
  );
}

/**
 * Links to the group's vault without duplicating the file browser. The Files
 * tab lists the items with the same roles.
 */
export function FilesTab({
  snapshot,
  store,
  onNavigate,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const items = itemCountOf(snapshot, store);
  return (
    <div className="roster">
      <Inset className="settings-inset">
        <InsetRow
          className="doorway"
          action={
            <Button
              variant="primary"
              icon="out"
              onClick={() => onNavigate({ kind: 'store', ref: store.id })}
            >
              Open in Files
            </Button>
          }
        >
          {/* Lead with the destination and use the Files folder mark,
              corresponding to the Chat marks used by channel rows. */}
          <span
            className="kico"
            style={{ background: 'var(--chip-bg)', color: 'var(--muted)' }}
            aria-hidden="true"
          >
            <Icon name="folder" />
          </span>
          <span className="t">
            <b>View {store.name}’s items in Files</b>
            <small>
              {items ? plural(items, 'item') : 'No items yet'} — the passwords
              and documents shared with this team. Permissions set here control
              who can access each item.
            </small>
          </span>
        </InsetRow>
      </Inset>
      <p className="fn">Files for this team are managed in the Files tab.</p>
    </div>
  );
}

/**
 * A group with incomplete setup has no members, channels, or items. This page
 * explains the state and provides the available recovery actions, including
 * the manual removal instructions.
 */
export function IncompleteGroupPage({
  snapshot,
  store,
  onFinish,
  onRemove,
  onCopyId,
  onNavigate,
}: {
  snapshot: AgentSnapshot;
  store: TeamStore;
  onFinish: () => void;
  onRemove: () => void;
  onCopyId: () => void;
  onNavigate: (location: Location) => void;
}): ReactNode {
  const server = serverOf(snapshot, store.id);
  const serverName = server ? serverDisplayName(server) : store.server;
  const account = snapshot.accounts.find(
    (candidate) =>
      candidate.alias === store.account && candidate.server === store.server,
  );
  // One sentence for this condition, wherever it is stated: the store page's
  // takeover reads the same copy, so the two cannot describe it differently.
  const copy = accessCopy('setup-incomplete', store, serverName);
  return (
    <div className="body">
      <div className="groups-wrap">
        <div className="roster">
          <Band
            label={copy.title}
            action={
              <Button
                variant="primary"
                size="sm"
                onClick={onFinish}
                disabled={copy.actionDisabled}
              >
                {copy.actionLabel ?? 'Finish setup'}
              </Button>
            }
          >
            {copy.detail}
          </Band>
          <SectionLabel>Local details</SectionLabel>
          <Inset>
            <InsetRow
              label="Account"
              action={
                <Button
                  size="sm"
                  icon="out"
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
              {account?.username ?? store.account} on {serverName}
            </InsetRow>
            <InsetRow
              label="Team ID"
              action={
                <Button size="sm" icon="copy" onClick={onCopyId}>
                  Copy
                </Button>
              }
            >
              <code title={store.team_id_hex}>
                {shortId(store.team_id_hex)}
              </code>
            </InsetRow>
          </Inset>
          {/* Finishing setup cannot succeed for every record that reaches this
              page — a name the server refused stays refused — so the page also
              offers the only other way out. */}
          <SectionLabel>Remove</SectionLabel>
          <Inset>
            <InsetRow
              label="Remove team"
              action={
                <Button size="sm" danger icon="trash" onClick={onRemove}>
                  Remove team…
                </Button>
              }
            >
              Drops this team’s saved identity and keys from this Mac. Use it
              when setup cannot be finished.
            </InsetRow>
          </Inset>
        </div>
      </div>
    </div>
  );
}
