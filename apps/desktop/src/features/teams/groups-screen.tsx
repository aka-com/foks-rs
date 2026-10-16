/**
 * A group's page: its header, the Members, Channels, Files and Settings tabs
 * and the sheets they open. The tabs and sheets are in `groups/`.
 */

import { groupDetailReadiness } from '../../model';
import { memberCountOf } from '../../model/team-members';
import { useGroupOperationController } from './groups/operation-controller';
import { useTabSheetState } from '../../navigation-guard';
import { UnfinishedActivity } from './invitations/unfinished-activity';
import { InviteNewUserSheet } from './invitations/invite-new-user-sheet';
import { InvitationActivitySheet } from './invitations/invitation-activity-sheet';
import { MembershipRequests } from './invitations/membership-requests';
import { useTeamRequestCounts } from '../../data/operation-queries';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Chip,
  Icon,
  MenuButton,
  MenuItem,
  Notice,
  tabId,
  tabPanelId,
  Tabs,
} from '../../components';
import {
  storeOperationAvailability,
  groupDetailFailure,
  partiesOf,
  plural,
  serverDisplayLabelForStore as displayServerName,
  storeDescriptionState,
  storeOf,
} from '../../model';
import type {
  AccountStore,
  AvailabilityOptions,
  FederationEntry,
  Item,
  Party,
} from '../../model';
import type { Bridge } from '../../bridge';
import { useSidebarInbox } from '../../chat/inbox-provider';
import { listChannels } from '../../chat/presentation';
import { GROUP_SETTINGS_TABS } from '../../location';
import type {
  GroupSettingsTab,
  Location,
  NavigateOptions,
} from '../../location';
import { markProfileRostersStale } from '../../data/roster-staleness';
import { PageHeader } from '../../shell/page-header';
import { useShellServices, useShellSnapshot } from '../../shell-services';
import { NewChatSheet } from '../chat';
import {
  ChannelsTab,
  FilesTab,
  IncompleteGroupPage,
  itemCountOf,
} from './group-tabs';
import { GroupMark } from '../../components/group-mark';
import { manageReason } from '../../model/group-model';
import { StoreAccessTakeover } from '../files';
import { useToast } from '../../../kit/toasts';
import { useExposureReviews } from './exposure-review-model';
import { ExposureReviewBands, ExposureReviewSheet } from './exposure-review';
import { GroupSheet } from './groups/group-sheet';
import type { GroupSheetKind } from './groups/group-sheet';
import { MembersTab } from './groups/members-tab';
import { SettingsTab } from './groups/settings-tab';
import { FederationRemovalSheet } from './groups/federation-removal-sheet';
import { AbandonGroupSheet } from './groups/abandon-group-sheet';

type Tab = GroupSettingsTab;
type Sheet = GroupSheetKind | null;

/** The base the group page's tab and panel ids are derived from. */
const GROUP_TABS = 'group-sections';
/** The `?state=` scene the page opens in: a tab, a sheet or an open menu. */
const stateName = (): string =>
  typeof window === 'undefined'
    ? ''
    : (new URLSearchParams(window.location.search).get('state') ?? '');

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
 * A team's or a share's settings page. It reads the snapshot, the bridge, the
 * shell's lease clock, which the Channels tab shares with Chat, the applied
 * refresh and the error handlers from the shell services.
 */
export function GroupSettingsScreen({
  location,
  accessGenerations,
  onNavigate,
  onOpenItem,
}: {
  location: Extract<Location, { kind: 'group-settings' }>;
  /** The shell's access generation per server, as a channel write reads it. */
  accessGenerations?: ReadonlyMap<string, number>;
  /**
   * `replace` marks a move within this page — the tab strip — which the
   * shell records in place of the last rather than behind it.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  /**
   * Opens an item in Files, from an exposure review. Without it, the review
   * lists items without an Open action.
   */
  onOpenItem?: (item: Item) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    accessNow,
    refresh: onSnapshotApplied,
    commandError: onError,
    mutationError: onSnapshotMutationError,
  } = useShellServices();
  const copy = useCopyText(bridge, onError);
  // Reviews of what removed or lowered members could read, kept on this
  // device after the change; one is open in its sheet at a time.
  const exposureReviews = useExposureReviews();
  const [exposureOpen, setExposureOpen] = useState<string | null>(null);
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
      [
        'add',
        'promote',
        'demote',
        'remove',
        'add-team',
        'party-remove',
      ].includes(initial)
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
  // A Create channel sheet whose submission is unresolved cannot be dismissed,
  // and a store switch must not take it away either: a submission has to be
  // settled where it was made.
  const channelUnresolved = useRef(false);
  // One clock for this render, the way the Chat tab reads its own: a cache
  // would freeze the availability decision at the moment the tab mounted, so
  // a check-in that lapses while the tab is open would never be noticed.
  const accessOptions: AvailabilityOptions = { nowSeconds: accessNow() };
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
    if (initial === 'promote')
      return parties.find((party) => party.username === 'dana.okafor') ?? null;
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
      setExposureOpen(null);
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
    !canReadRoster ||
    rosterFailure ||
    federationFailure ||
    groupDetailReadiness(snapshot, store.id, 'roster') !== 'ready' ||
    groupDetailReadiness(snapshot, store.id, 'federation') !== 'ready'
      ? null
      : plural(memberCount, 'member'),
  ].filter(Boolean);
  // The channels this group's inbox entry holds, or `undefined` while the
  // service has not answered for it. A hidden conversation is listed on the
  // tab but not counted on the strip, the way the Create channel sheet counts.
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
      membership={{ operations: membershipPending, onResume: resumeMembership }}
      onClose={() => setReviewing(false)}
      onComplete={() => onApplied('Team requests updated', store.server)}
    />
  ) : null;
  const exposureReview = exposureReviews.find(
    (review) => review.id === exposureOpen && review.scope === store.id,
  );
  const exposureSheet = exposureReview ? (
    <ExposureReviewSheet
      snapshot={snapshot}
      review={exposureReview}
      onOpenItem={
        onOpenItem
          ? (item) => {
              setExposureOpen(null);
              onOpenItem(item);
            }
          : undefined
      }
      onClose={() => setExposureOpen(null)}
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
          {/* Combine blocking membership operations and invitation activity
              in one review entry point. */}
          {reviewing ? null : (
            <UnfinishedActivity
              bridge={bridge}
              store={store}
              membership={membershipPending}
              invitations={canManageRoster}
              onError={onError}
              onReview={() => setReviewing(true)}
            />
          )}
          <ExposureReviewBands
            reviews={exposureReviews.filter(
              (review) => review.scope === store.id,
            )}
            onReview={setExposureOpen}
          />
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
                    ...(!canReadRoster ||
                    rosterFailure ||
                    federationFailure ||
                    groupDetailReadiness(snapshot, store.id, 'roster') !==
                      'ready' ||
                    groupDetailReadiness(snapshot, store.id, 'federation') !==
                      'ready'
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
              snapshot={snapshot}
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
          {/* Creating a channel is the Create channel sheet, aimed at this
              group: one preparation, recovered where it was made. */}
          {addingChannel ? (
            <NewChatSheet
              snapshot={snapshot}
              bridge={bridge}
              team={store.id}
              accessOptions={accessOptions}
              accessNow={accessNow}
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
      {exposureSheet}
    </>
  );
}
