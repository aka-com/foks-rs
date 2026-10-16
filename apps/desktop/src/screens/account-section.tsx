/**
 * Settings › Account: one account's profile, the account the address's
 * `store` names and the rail header shows.
 *
 * The account is the section's header: its mark, its username as the title,
 * and the server and the server's trust state on the line under it — the
 * shape the team page already uses for a team. The header has no switcher:
 * the rail header's avatar menu is the application's only account switcher,
 * and it sits beside this section. The body displays primary account
 * properties with management links: Username, Shown as (the local alias),
 * Server, Devices, and Teams. Secondary integrations (Bot accounts, web
 * administration, SSO, FOKS CLI import, account recovery, and hardware-backed
 * account creation) render after the device-wide server inventory. The
 * selected account's passphrase is managed here as well.
 * Alerts without a navigable destination, such as catalog read failures,
 * render above the details list; entity-specific alerts render in their
 * corresponding Settings or Teams views.
 *
 * The section draws its own header and tab panel, because its header is an
 * identity rather than the section's name, which is what the other sections'
 * shared header draws.
 */

import { inventoryReadiness } from '../model';
import { CollectionStatus } from '../components/collection-status';
import { useToast } from '../../kit/toasts';
import { synchronizeApplied } from '../data/operation-outcome';
import { useDeviceMetadata } from '../data/device-cache';
import { workflowAvailability } from '../model/workflow-availability';
import { LocalAliasPanel } from '../components/local-alias-panel';
import { localAliasOf } from '../model';
import { useTabSheetState } from '../navigation-guard';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { AdminPanel } from '../components/admin-panel';
import { AssistantClockBand } from '../components/assistant-clock-band';
import { BotPanel } from '../components/bot-panel';
import { RenamePanel } from '../components/rename-panel';
import { RenameUnlockSheet } from '../components/rename-banner';
import { useRenameStatus } from '../rename-status';
import type { RenameAction } from '../rename-contract';
import { SsoPanel } from '../components/sso-panel';
import { Band, Button, Chip, Inset, InsetRow } from '../components';
import type { Bridge } from '../bridge';
import {
  accountStopped,
  accountStores,
  serverDisplayLabel,
  serverDisplayLabelForStore,
  storeDescription,
  usernameOf,
} from '../model';
import type { AgentSnapshot, Notification } from '../model';
import type { Server } from '../model';
import type { Location, NavigateOptions } from '../location';
import type { MutationFailureHandler } from '../data/mutation-recovery';
import { PageHeader } from '../shell/page-header';
import { AccountMark } from '../components/account-mark';
import { GoProfileConnectSheet } from './go-profile-connect';
import { canRetry, unroutedNotices } from './unrouted-notices';
import {
  EnrollSheet,
  PassphraseSheet,
  RecoverSheet,
} from '../features/devices';
import { AccountPanel } from './settings/account-panel';
import type { AccountSheet } from './settings/account-panel';
import { AccountActions } from './settings/account-actions';
import {
  NoAvailableAccount,
  UnavailableAccount,
} from './settings/unavailable-account';

const ACTION_UNAVAILABLE =
  'Resolve this under Account servers, or in the team’s settings.';

interface UnroutedNoticesProps {
  snapshot: AgentSnapshot;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
}

/**
 * Notices this page cannot route anywhere else: a catalog read that only a
 * retry can fix, or a note whose place could not be resolved — an alias that
 * matches more than one store, or a federated membership this Mac cannot place. Every
 * other note already has a home: a lapsed or unverified server shows on
 * Settings › Account, a team whose setup is incomplete shows on Teams, and a
 * team's membership in another team shows as Inactive on the host team's own
 * Members page. Drawing those again here would be a second copy of a state
 * the reader can already see where it is acted on.
 */
function UnroutedNotices({
  snapshot,
  onRefreshSnapshot,
  onError,
}: UnroutedNoticesProps): ReactNode {
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const notes = unroutedNotices(snapshot);
  if (!notes.length) return null;

  const retry = (note: Notification): void => {
    if (!canRetry(note) || busy.has(note.id)) return;
    setBusy((current) => new Set([...current, note.id]));
    onRefreshSnapshot()
      .catch(onError)
      .finally(() => {
        setBusy((current) => {
          const next = new Set(current);
          next.delete(note.id);
          return next;
        });
      });
  };

  return (
    <section className="people-attention" aria-label="Needs attention">
      {notes.map((note) => (
        <Band
          key={note.id}
          severity={note.severity}
          label={note.title}
          action={
            canRetry(note) ? (
              <Button
                variant="primary"
                size="sm"
                disabled={busy.has(note.id)}
                title="Retry loading"
                onClick={() => retry(note)}
              >
                {note.action}
              </Button>
            ) : note.action ? (
              <Chip tone="warn" title={ACTION_UNAVAILABLE}>
                {note.action}
              </Chip>
            ) : null
          }
        >
          {note.detail}
        </Band>
      ))}
    </section>
  );
}

export interface AccountSectionProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The Settings address this section is open at; `store` is the account. */
  location: Extract<Location, { kind: 'settings' }>;
  /**
   * The id the sub-navigation's Account tab points at, and the tab's own id,
   * so the panel this section draws is the one the tab controls.
   */
  panel: { id: string; labelledBy: string };
  /**
   * `force` marks the replacement this page makes on its own behalf — the
   * default route resolving to an account's exact `StoreRef` — which is the
   * page already on screen writing its own address and not a move any panel
   * may answer for.
   */
  onNavigate: (location: Location, options?: NavigateOptions) => void;
  /**
   * Refreshes data after a mutation. When a profile is specified, only that
   * server profile is reloaded instead of refreshing the full catalog.
   */
  onRefresh: (message: string, profile?: string) => Promise<void>;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
  /** Device-wide server inventory, composed after every account state. */
  serverSection?: ReactNode;
}

export function AccountSection({
  snapshot,
  bridge,
  location,
  panel,
  onNavigate,
  onRefresh,
  onRefreshSnapshot,
  onError,
  onMutationError,
  serverSection,
}: AccountSectionProps): ReactNode {
  const toasts = useToast();
  const renameComplete = async (profile: string): Promise<void> => {
    const result = await synchronizeApplied(() =>
      onRefresh('Username updated', profile),
    );
    if (result.synchronization === 'pending') {
      toasts.show('Username updated. Refresh pending.');
      onError(result.error);
    }
  };
  const stores = accountStores(snapshot);
  // Select by exact StoreRef: two servers may both hold an account aliased
  // `personal`, so an alias would not say which one an address means.
  const requested = location.store;
  const selected = requested
    ? stores.find((store) => store.id === requested)
    : stores[0];
  const unavailable = requested !== undefined && selected === undefined;
  const [sheet, setSheet] = useTabSheetState<
    AccountSheet | 'go-profile' | null
  >('settings.account.sheet', null, false);
  const [pairingProfile, setPairingProfile] = useState<Server | undefined>();
  // Keep pending renames on the account page after preparation closes.
  const renameAllowed =
    !!selected &&
    workflowAvailability(snapshot, 'account-rename', {
      profile: selected.server,
      account: selected.account,
    }).available;
  const rename = useRenameStatus({
    bridge,
    profile: selected?.server,
    account: selected?.account,
    enabled: renameAllowed,
  });
  const [unlock, setUnlock] = useState<RenameAction | null>(null);
  // What this account holds: the keys are read here because the profile's
  // Devices row counts them, not because anything on this page acts on one.
  const stopped = selected
    ? accountStopped(snapshot, selected)
    : { stopped: true, reason: '' };
  const keysStopped =
    !selected ||
    !workflowAvailability(snapshot, 'devices-list', {
      profile: selected.server,
      account: selected.account,
    }).available;
  const {
    lists,
    loading: loadingKeys,
    failed: keysFailed,
    freshness,
    retry: retryMetadata,
  } = useDeviceMetadata({
    bridge,
    snapshot,
    profile: selected?.server,
    store: selected?.id,
    enabled: !keysStopped,
    recovery: { refresh: onRefreshSnapshot },
    onError,
  });

  // Normalize the route to the active account's StoreRef. Because the
  // destination matches the current location, navigation guards are bypassed
  // and active dialogs remain open.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate(
      { ...location, section: 'account', store: selected.id },
      { force: true },
    );
  }, [location, onNavigate, selected]);

  // Changing account identity resets input state; query data is selected by
  // exact account/profile identity on the same render, without copying it.
  useEffect(() => {
    setSheet(null);
  }, [selected?.id, setSheet]);

  // The page's header is the account the page is about: its mark, its username
  // as the title, and the server on the line under it. The branches that name no account — an address
  // naming one this Mac no longer holds, or no account at all — have no
  // identity to state, so they keep the tab's own name as the title.
  //
  // Whether access is stopped is a fact about the header's subject rather than
  // part of the account's name or of its server line, so it is the header's
  // own right-hand mark: at the end of the row, centred against both lines,
  // instead of trailing the server on the second one.
  const headerUsername = selected ? usernameOf(snapshot, selected) : undefined;
  const identity = selected
    ? {
        title: headerUsername ?? 'Identity unavailable',
        mark: (
          <AccountMark name={headerUsername ?? selected.account} size="round" />
        ),
        sub: <span>{serverDisplayLabelForStore(snapshot, selected)}</span>,
        state: stopped.stopped ? (
          <Chip tone="warn">{storeDescription(snapshot, selected)}</Chip>
        ) : undefined,
      }
    : undefined;

  // The fallback band belongs above the facts of the account the page names.
  // The branches that state no identity draw no facts either, so they keep it
  // at the top of the page.
  const notices = (
    <UnroutedNotices
      snapshot={snapshot}
      onRefreshSnapshot={onRefreshSnapshot}
      onError={onError}
    />
  );
  const unpairedServers =
    snapshot.profileInventoryStatus === 'complete'
      ? snapshot.servers.filter(
          (server) =>
            server.accounts.length === 0 &&
            snapshot.profileInventory.find(
              (inventory) => inventory.profile === server.profileName,
            )?.accounts === 'complete',
        )
      : [];
  const noticesAndUnpairedServers = (
    <>
      {notices}
      {unpairedServers.map((server) => (
        <Inset key={server.profileName}>
          {/* The server's name is the value, not the label: the label column
              is a fixed 88px and a hostname overran it. */}
          <InsetRow
            label="Server"
            action={
              <Button
                onClick={() => {
                  setPairingProfile(server);
                  setSheet('go-profile');
                }}
              >
                Pair
              </Button>
            }
          >
            {serverDisplayLabel(server)}{' '}
            <span className="dim">Connected, not paired</span>
          </InsetRow>
        </Inset>
      ))}
    </>
  );

  return (
    <>
      <PageHeader
        ruled
        title={identity?.title ?? 'Account'}
        mark={identity?.mark}
        sub={identity?.sub}
        action={identity?.state}
      />
      <div
        className="body"
        role="tabpanel"
        id={panel.id}
        aria-labelledby={panel.labelledBy}
      >
        <div className="settings-main account-main">
          {!selected && inventoryReadiness(snapshot, 'accounts') !== 'ready' ? (
            <>
              <CollectionStatus
                state={inventoryReadiness(snapshot, 'accounts')}
                label="accounts"
              />
              <Button onClick={() => void onRefreshSnapshot().catch(onError)}>
                Refresh
              </Button>
            </>
          ) : unavailable ? (
            <>
              {noticesAndUnpairedServers}
              <UnavailableAccount
                stores={stores}
                snapshot={snapshot}
                onSelect={(store) =>
                  onNavigate({
                    kind: 'settings',
                    section: 'account',
                    store: store.id,
                  })
                }
                onRefresh={() => void onRefreshSnapshot().catch(onError)}
              />
            </>
          ) : selected ? (
            <>
              <AccountPanel
                notices={
                  <>
                    {noticesAndUnpairedServers}
                    <AssistantClockBand
                      bridge={bridge}
                      profile={selected.server}
                      account={selected.account}
                    />
                  </>
                }
                freshness={freshness}
                onRetry={retryMetadata}
                snapshot={snapshot}
                store={selected}
                lists={lists}
                loading={loadingKeys}
                failed={keysFailed}
                stopped={stopped}
                rename={rename}
                onRenameUnlock={(action) => {
                  setUnlock(action);
                  setSheet('rename-unlock');
                }}
                onRenameComplete={() => renameComplete(selected.server)}
                bridge={bridge}
                onNavigate={onNavigate}
                onSheet={(next) => {
                  // The import sheet opened from the account's own line adds an
                  // account; a server row's Pair fills `pairingProfile` instead.
                  if (next === 'go-profile') setPairingProfile(undefined);
                  setSheet(next);
                }}
              />
            </>
          ) : (
            <>
              {noticesAndUnpairedServers}
              <NoAvailableAccount
                onConnectGoProfile={() => setSheet('go-profile')}
              />
            </>
          )}
          {serverSection}
          {selected ? (
            <AccountActions
              snapshot={snapshot}
              store={selected}
              onSheet={(next) => {
                if (next === 'go-profile') setPairingProfile(undefined);
                setSheet(next);
              }}
            />
          ) : null}
        </div>
      </div>
      {sheet === 'go-profile' ? (
        <GoProfileConnectSheet
          bridge={bridge}
          existingProfile={pairingProfile}
          onAdded={async (profile) => {
            setSheet(null);
            await onRefresh(
              'Server connected. Pair an account to continue.',
              profile,
            );
          }}
          onClose={() => setSheet(null)}
          onConnected={async (profile, alias) => {
            setSheet(null);
            await onRefresh(
              `Connected account "${alias}" from FOKS CLI`,
              profile,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {selected && sheet === 'local-alias' ? (
        <LocalAliasPanel
          key={selected.id}
          bridge={bridge}
          store={selected.id}
          alias={localAliasOf(snapshot, selected)}
          presentation={{
            title: 'Change local alias',
            onClose: () => setSheet(null),
          }}
          onComplete={async () => {
            // Close the sheet immediately after saving without waiting for the
            // background catalog refresh to prevent blocking user navigation.
            setSheet(null);
            await onRefresh('Local alias updated', selected.server);
          }}
        />
      ) : null}
      {selected && sheet === 'rename' ? (
        <RenamePanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Change username',
            onClose: () => setSheet(null),
          }}
          onPrepared={(rows) => {
            // Store the prepared operation before displaying its actions.
            setSheet(null);
            rename.learn(rows);
          }}
        />
      ) : null}
      {selected && sheet === 'rename-unlock' && unlock ? (
        <RenameUnlockSheet
          profile={selected.server}
          account={selected.account}
          action={unlock}
          status={rename}
          presentation={{
            title: 'Unlock security key',
            onClose: () => setSheet(null),
          }}
          onDone={async (completed) => {
            setSheet(null);
            if (completed) await renameComplete(selected.server);
          }}
        />
      ) : null}
      {selected && sheet === 'passphrase' ? (
        <PassphraseSheet
          store={selected}
          onClose={() => setSheet(null)}
          onDone={(message) => {
            setSheet(null);
            void onRefresh(message, selected.server).catch((error) =>
              onMutationError(error),
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {selected && sheet === 'bot' ? (
        <BotPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Bot accounts',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('Bot account updated', selected.server)}
        />
      ) : null}
      {selected && sheet === 'admin' ? (
        <AdminPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          presentation={{
            title: 'Web admin panel',
            onClose: () => setSheet(null),
          }}
        />
      ) : null}
      {selected && sheet === 'sso' ? (
        <SsoPanel
          bridge={bridge}
          profile={selected.server}
          account={selected.account}
          login={true}
          presentation={{
            title: 'Sign in via SSO',
            onClose: () => setSheet(null),
          }}
          onComplete={() => onRefresh('SSO sign-in verified', selected.server)}
        />
      ) : null}
      {selected && sheet === 'recover' ? (
        <RecoverSheet
          store={selected}
          onClose={() => setSheet(null)}
          onDone={async () => {
            setSheet(null);
            await onRefresh(
              'Recovery submitted successfully.',
              selected.server,
            );
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
      {selected && sheet === 'enroll' ? (
        <EnrollSheet
          store={selected}
          card={snapshot.cardsConnected[0]}
          onClose={() => setSheet(null)}
          onDone={async () => {
            setSheet(null);
            await onRefresh('Hardware-key account created.', selected.server);
          }}
          onError={(error) => void onMutationError(error)}
        />
      ) : null}
    </>
  );
}
