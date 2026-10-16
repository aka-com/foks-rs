/**
 * Settings › Account's account panel: the account's facts, its teams and
 * devices, and the workflows that act on it.
 */

import { inventoryReadiness } from '../../../model';
import { CollectionStatus } from '../../../components/collection-status';
import type { metadataFreshness } from '../../../data/device-cache';
import { useWorkflowAccess } from '../../../workflow-context';
import { FreshnessCaption } from '../../../components/metadata-status';
import { localAliasOf } from '../../../model';
import type { ReactNode } from 'react';
import { RenameBanner } from './rename-banner';
import type { RenameStatus } from './rename-status';
import type { RenameAction } from '../../../rename-contract';
import { Band, Button, Icon, Inset, InsetRow } from '../../../components';
import type { Bridge } from '../../../bridge';
import {
  plural,
  serverDisplayLabelForStore,
  storeDescription,
  storeNavigationOrder,
  storeOperationAvailability,
  usernameOf,
} from '../../../model';
import type { AccountStore, AgentSnapshot, TeamStore } from '../../../model';
import type { Location } from '../../../location';
import { storeUsageText, useStoreUsage } from '../../../store-usage';
import { deviceEntries } from '../../../data/device-model';
import type { DeviceLists } from '../../../data/device-model';

/** The account panels reached from a row on this page. */
export type AccountSheet =
  | 'local-alias'
  | 'rename'
  | 'rename-unlock'
  | 'passphrase'
  | 'bot'
  | 'admin'
  | 'sso'
  | 'recover'
  | 'enroll'
  | 'go-profile';

/** The teams this account belongs to, as the catalog holds them. */
function teamsOnAccount(
  snapshot: AgentSnapshot,
  store: AccountStore,
): TeamStore[] {
  return storeNavigationOrder(snapshot).filter(
    (candidate): candidate is TeamStore =>
      candidate.kind === 'team' &&
      candidate.server === store.server &&
      candidate.account === store.account,
  );
}

/** The chosen account: who it is, and the facts and workflows that act on it. */
export function AccountPanel({
  snapshot,
  bridge,
  store,
  lists,
  loading,
  failed,
  stopped,
  rename,
  onRenameUnlock,
  onRenameComplete,
  onNavigate,
  onSheet,
  notices,
  freshness,
  onRetry,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  store: AccountStore;
  /** Pending rename state. */
  rename: RenameStatus;
  onRenameUnlock: (action: RenameAction) => void;
  onRenameComplete: () => void | Promise<void>;
  /** The keys this account holds, once the agent has answered. */
  lists: DeviceLists;
  /** Whether the keys behind the counts are current, stale or on their way. */
  freshness: ReturnType<typeof metadataFreshness>;
  onRetry: () => void;
  loading: boolean;
  /** Whether the read failed, which is not the same as holding no keys. */
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  onNavigate: (location: Location) => void;
  onSheet: (sheet: AccountSheet) => void;
  /** The notices no tab can resolve, drawn above the facts they qualify. */
  notices: ReactNode;
}): ReactNode {
  const username = usernameOf(snapshot, store);
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  const devicesAccess = access.props('devices-list', target);
  const devicesStopped = {
    stopped: devicesAccess.disabled,
    reason: devicesAccess.title,
  };
  const deviceCount = deviceEntries(lists).length;
  const teams = teamsOnAccount(snapshot, store);
  const server = snapshot.servers.find(
    (entry) => entry.profileName === store.server,
  );
  // The account this page is about is named by the page's own header, so the
  // body opens on the facts rather than stating the identity a second time.
  return (
    <>
      {stopped.stopped ? (
        <Band
          severity="crit"
          label="Account access is stopped"
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
          Remote operations are unavailable until a connection is reestablished.
        </Band>
      ) : null}
      {notices}
      <RenameBanner
        status={rename}
        profile={store.server}
        account={store.account}
        onUnlock={onRenameUnlock}
        onComplete={onRenameComplete}
      />
      <Inset
        className={`settings-inset middle wide${freshness.stale ? ' stale' : ''}`}
      >
        <InsetRow
          label="Username"
          action={
            <>
              {/* Resolve the pending operation before preparing another. */}
              <Button
                size="sm"
                {...(rename.pending
                  ? {
                      disabled: true,
                      title:
                        'Confirm or discard the pending username change first.',
                    }
                  : access.props('account-rename', target))}
                onClick={() => onSheet('rename')}
              >
                Change…
              </Button>
              <Button
                size="sm"
                {...access.props('passphrase', target)}
                onClick={() => onSheet('passphrase')}
              >
                Passphrase…
              </Button>
            </>
          }
        >
          <b>{username ?? 'Identity unavailable'}</b>
        </InsetRow>
        <InsetRow
          label="Shown as"
          action={
            <Button
              size="sm"
              aria-label="Change local alias"
              {...access.props('local-alias', target)}
              onClick={() => onSheet('local-alias')}
            >
              Change…
            </Button>
          }
        >
          {localAliasOf(snapshot, store)}
        </InsetRow>
        <InsetRow
          label="Server"
          action={
            <Button
              size="sm"
              className="account-fact-link"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
                  profile: store.server,
                  store: store.id,
                })
              }
            >
              Server details ›
            </Button>
          }
        >
          {/* The server names itself; its state is a quieter second phrase
              after it, not a clause of equal weight behind a middot. The two
              share one element because this inset stacks its value's children,
              and the state belongs on the name's own line. */}
          <span>
            {serverDisplayLabelForStore(snapshot, store)}
            {stopped.stopped ? (
              <>
                {' '}
                <span className="dim">{storeDescription(snapshot, store)}</span>
              </>
            ) : server?.trust.status === 'verified' ? (
              <span className="verified-mark" role="img" aria-label="Verified">
                <Icon name="circleCheck" />
              </span>
            ) : null}
          </span>
        </InsetRow>
        <InsetRow
          label="Devices"
          action={
            <Button
              size="sm"
              className="account-fact-link"
              disabled={devicesStopped.stopped}
              title={devicesStopped.stopped ? devicesStopped.reason : undefined}
              onClick={() => onNavigate({ kind: 'devices', store: store.id })}
            >
              Devices ›
            </Button>
          }
        >
          {devicesStopped.stopped
            ? 'Not listed while access is stopped'
            : loading
              ? 'Reading this account’s keys…'
              : failed
                ? 'Devices and keys could not be read.'
                : plural(deviceCount, 'device and key', 'devices and keys')}
        </InsetRow>
        <InsetRow
          label="Teams"
          action={
            <Button
              size="sm"
              className="account-fact-link"
              onClick={() => onNavigate({ kind: 'teams', store: store.id })}
            >
              Teams ›
            </Button>
          }
        >
          {inventoryReadiness(snapshot, 'teams', store.server) === 'ready' ? (
            plural(teams.length, 'team')
          ) : (
            <CollectionStatus
              state={inventoryReadiness(snapshot, 'teams', store.server)}
              label="teams"
            />
          )}
        </InsetRow>
        <StorageRow
          bridge={bridge}
          snapshot={snapshot}
          store={store}
          stopped={stopped.stopped}
        />
      </Inset>
      <FreshnessCaption
        label="device metadata"
        freshness={freshness}
        onRetry={onRetry}
      />
    </>
  );
}

/**
 * What the account's own vault stores on its server. It is read when this page
 * opens, not on the metadata cadence; the figure has no denominator because
 * the server reports none.
 */
function StorageRow({
  bridge,
  snapshot,
  store,
  stopped,
}: {
  bridge: Bridge;
  snapshot: AgentSnapshot;
  store: AccountStore;
  stopped: boolean;
}): ReactNode {
  const access = storeOperationAvailability(snapshot, store, 'vault');
  const readable = !stopped && access.available;
  const usage = useStoreUsage(bridge, snapshot, store, readable);
  return (
    <InsetRow label="Storage">
      {stopped
        ? 'Not read while access is stopped'
        : readable
          ? storeUsageText(usage)
          : !access.available && access.reason === 'loading'
            ? 'Reading storage used…'
            : 'Storage used unavailable.'}
    </InsetRow>
  );
}
