/** Device-wide server inventory and server details embedded in Settings › Account. */

import { inventoryReadiness } from '../../../model';
import { CollectionStatus } from '../../../components/collection-status';
import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '../../../../kit/toasts';
import type {
  Bridge,
  CheckedServer,
  ServerStatusSnapshot,
} from '../../../bridge';
import { useServerMetadata } from './use-server-metadata';
import {
  expires,
  isLocked,
  leaseExpiry,
  pinnedHost,
  resolveServerUiState,
  serverFor,
  versionMismatchText,
} from './server-model';
import type { PinnedHost } from './server-model';
import { ServerList } from './server-list';
import { ServerMark, StatusBand, StatusChip } from './server-status';
import {
  Band,
  Button,
  Chip,
  Inset,
  InsetRow,
  SectionLabel,
} from '../../../components';
import type { Location } from '../../../location';
import { useSheetGuard } from '../../../navigation-guard';
import {
  plural,
  serverDisplayLabelOrLoading,
  shortId,
  storeAttentionState,
  storeDescription,
  teamCaption,
} from '../../../model';
import type { MutationFailureHandler } from '../../../data/mutation-recovery';
import type {
  Server,
  StoreRef,
  TeamStore,
  AgentSnapshot,
} from '../../../model';
import { PageHeader } from '../../../shell/page-header';
import { AccountMark } from '../../../components/account-mark';
import { ProfileKeys } from '../../devices';
import { GroupMark } from '../../../components/group-mark';
import { matchesShortcut } from '../../../shortcuts';
import { AddServerSheet } from './add-server-sheet';
import { RenameServerSheet } from './rename-server-sheet';
import { RemoveServerSheet } from './remove-server-sheet';

interface Props {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The server the section is open on, or nothing for the list. */
  profile?: string;
  /** The Account page to retain while opening and closing server details. */
  store?: StoreRef;
  /** The named fixture scene Settings was entered at, captured once there. */
  scene: string;
  /** Account tab panel identity, present when one server owns the page. */
  panel?: { id: string; labelledBy: string };
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
  onMutationError: MutationFailureHandler;
}

type Sheet = 'add' | 'rename' | 'remove' | null;

/** Returns the Account location for the server list or one server detail. */
const servers = (profile?: string, store?: StoreRef): Location => ({
  kind: 'settings',
  section: 'account',
  ...(store ? { store } : {}),
  ...(profile ? { profile } : {}),
});

export function ServersSection({
  snapshot: agentSnapshot,
  bridge,
  profile,
  store,
  scene,
  panel,
  onNavigate,
  onRefresh,
  onError,
  onMutationError,
}: Props): ReactNode {
  const [enteredScene] = useState(scene);
  const [sheet, setSheet] = useState<Sheet>(() =>
    enteredScene === 'servers-add'
      ? 'add'
      : enteredScene === 'servers-reset'
        ? 'remove'
        : null,
  );
  const toasts = useToast();
  const selected = serverFor(agentSnapshot, profile);
  const { statuses, checked, busy, check } = useServerMetadata({
    bridge,
    snapshot: agentSnapshot,
    profile,
    enteredScene,
    onMutationError,
    onRefresh,
    toast: (message) => toasts.show(message),
  });

  // A check pins or advances the server's identity and then reads its signed
  // status back; the section is where both answers are stated. Nothing else
  // here is typed, so this is the only thing the section answers for.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the server check to finish.' }
      : null,
  );

  const currentHost = selected
    ? (checked.get(selected.profileName) ?? pinnedHost(selected))
    : null;
  const selectedState = selected
    ? resolveServerUiState(agentSnapshot, selected)
    : null;
  const selectedAccount = selected
    ? agentSnapshot.accounts.find(
        (item) => item.server === selected.profileName,
      )
    : undefined;

  // Keyboard shortcut: ⌘R / Ctrl+R triggers a check on the selected server.
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (
        event.defaultPrevented ||
        event.target instanceof HTMLInputElement ||
        event.target instanceof HTMLTextAreaElement
      )
        return;
      if (matchesShortcut(event, 'serverCheck') && selected) {
        event.preventDefault();
        void check(selected);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

  const overlay =
    sheet === 'add' ? (
      <AddServerSheet
        bridge={bridge}
        onClose={() => setSheet(null)}
        onAdded={async (added) => {
          setSheet(null);
          await onRefresh('Server verified and added.');
          onNavigate(servers(added, store));
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : sheet === 'rename' && selected ? (
      <RenameServerSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onRenamed={async () => {
          await onRefresh('Server name updated.');
          setSheet(null);
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : sheet === 'remove' && selected ? (
      <RemoveServerSheet
        server={selected}
        bridge={bridge}
        onClose={() => setSheet(null)}
        onRemoved={async () => {
          setSheet(null);
          onNavigate(servers(undefined, store));
          await onRefresh(
            `Removed ${serverDisplayLabelOrLoading(selected)} and its credentials`,
          );
        }}
        onError={(error) => void onMutationError(error)}
      />
    ) : null;

  const content = selected ? (
    <>
      <ServerBody
        snapshot={agentSnapshot}
        server={selected}
        status={statuses.get(selected.profileName)}
        host={currentHost}
        checked={checked.get(selected.profileName)}
        busy={busy}
        onCheck={() => void check(selected)}
        onRemove={() => setSheet('remove')}
        onCopy={(text) =>
          void bridge
            .copyText(text)
            .then(() => toasts.show('Host ID copied to clipboard'))
            .catch(onError)
        }
        onOpenGroup={(store) => onNavigate({ kind: 'store', ref: store.id })}
        onOpenAccount={(store) =>
          onNavigate({ kind: 'settings', section: 'account', store })
        }
      />
      <ProfileKeys
        key={selected.profileName}
        snapshot={agentSnapshot}
        server={selected}
        bridge={bridge}
        onError={onError}
      />
    </>
  ) : (
    <ServerList
      snapshot={agentSnapshot}
      statuses={statuses}
      busy={busy}
      onOpen={(next) => onNavigate(servers(next, store))}
      onCheck={(server) => void check(server)}
      onAdd={() => setSheet('add')}
    />
  );

  if (selected && selectedState)
    return (
      <>
        <PageHeader
          ruled
          title={serverDisplayLabelOrLoading(selected)}
          mark={<ServerMark state={selectedState} />}
          sub={
            selectedAccount && !isLocked(selectedState)
              ? `Signed in as ${selectedAccount.username}`
              : selected.configuredEndpoint
          }
          action={
            <>
              <StatusChip state={selectedState} />
              <Button size="sm" onClick={() => setSheet('rename')}>
                Rename…
              </Button>
              <Button
                size="sm"
                icon="refresh"
                disabled={
                  busy ||
                  selectedState === 'blocked' ||
                  selectedState === 'recovery-required'
                }
                title={
                  selectedState === 'recovery-required'
                    ? 'Restore saved security state before checking this identity.'
                    : selectedState === 'blocked'
                      ? 'This server is blocked because its identity changed'
                      : 'Verify the saved server identity; this does not renew compatibility permission.'
                }
                onClick={() => void check(selected)}
              >
                Check
              </Button>
            </>
          }
        />
        <div
          className="body"
          role="tabpanel"
          id={panel?.id}
          aria-labelledby={panel?.labelledBy}
        >
          <div className="settings-main">{content}</div>
        </div>
        {overlay}
      </>
    );

  return (
    <>
      {content}
      {overlay}
    </>
  );
}

function ServerBody({
  snapshot: agentSnapshot,
  server,
  status,
  host,
  checked,
  busy,
  onCheck,
  onRemove,
  onCopy,
  onOpenGroup,
  onOpenAccount,
}: {
  snapshot: AgentSnapshot;
  server: Server;
  /** The signed status this page checked itself; see `leaseExpiry`. */
  status?: ServerStatusSnapshot;
  host: PinnedHost | null;
  checked?: CheckedServer;
  busy: boolean;
  onCheck: () => void;
  onRemove: () => void;
  onCopy: (text: string) => void;
  onOpenGroup: (store: TeamStore) => void;
  /** The account on this server, on Account, where an account is managed. */
  onOpenAccount: (store: StoreRef) => void;
}): ReactNode {
  const state = resolveServerUiState(agentSnapshot, server);
  const locked = isLocked(state);
  const blocked = state === 'blocked';
  const expiry = leaseExpiry(server, status);
  const account = agentSnapshot.accounts.find(
    (item) => item.server === server.profileName,
  );
  // Teams that need attention — setup incomplete, and the like — list after
  // the ones in a normal state, so the working teams read first.
  const groups = agentSnapshot.stores
    .filter(
      (store): store is TeamStore =>
        store.kind === 'team' && store.server === server.profileName,
    )
    .sort(
      (a, b) =>
        Number(storeAttentionState(agentSnapshot, a) !== 'normal') -
        Number(storeAttentionState(agentSnapshot, b) !== 'normal'),
    );
  const hasHost = Boolean(host) && state !== 'unavailable';

  return (
    <>
      <StatusBand
        state={state}
        expiry={expiry}
        busy={busy}
        onCheck={onCheck}
        onRemove={onRemove}
      />
      {checked?.serverVersion && !checked.serverVersion.compatible ? (
        <Band severity="warn" label="Version mismatch">
          {versionMismatchText(checked.serverVersion)}
        </Band>
      ) : null}

      <Inset className="settings-inset middle">
        {state === 'checked' ? (
          <>
            {/* The header carries the one Check this page offers; a checked
                server's status row states the fact and nothing else. */}
            <InsetRow label="Status">
              {checked
                ? 'Last checked now. No issues.'
                : 'Identity pinned on this device.'}
            </InsetRow>
            <InsetRow label="Expires">{expires(expiry)}</InsetRow>
          </>
        ) : state === 'pending' ? (
          <InsetRow label="Status">Verifying server status…</InsetRow>
        ) : state === 'unprobed' ? (
          <InsetRow label="Expires">
            <span className="stopped">Not verified yet</span>
          </InsetRow>
        ) : state === 'lapsed' ? (
          <InsetRow label="Expired">
            <b className="danger-title">{expires(expiry)}</b>
          </InsetRow>
        ) : state === 'unavailable' ? (
          <InsetRow label="Expires">
            <span className="stopped">Unknown</span>
          </InsetRow>
        ) : (
          <InsetRow label="Expires">
            <span className="stopped">Not read while locked</span>
          </InsetRow>
        )}
      </Inset>

      {/* What stops if this server lapses: the account held on it, and the
          groups that live there. */}
      <SectionLabel>Accounts on this server</SectionLabel>
      <Inset className="settings-inset middle wide">
        {locked ? (
          <InsetRow label="You">
            <span className="stopped">Hidden while locked</span>
          </InsetRow>
        ) : account ? (
          <InsetRow
            className="devrow"
            action={
              <Button size="sm" onClick={() => onOpenAccount(account.store)}>
                Account
              </Button>
            }
          >
            <AccountMark name={account.username} />
            <span className="t">
              <span className="namechip">
                <b>{account.username}</b> <Chip>{account.alias}</Chip>
              </span>
            </span>
          </InsetRow>
        ) : inventoryReadiness(
            agentSnapshot,
            'accounts',
            server.profileName,
          ) !== 'ready' ? (
          <CollectionStatus
            state={inventoryReadiness(
              agentSnapshot,
              'accounts',
              server.profileName,
            )}
            label="accounts"
          />
        ) : (
          <InsetRow label="You">
            No account on this server<small>Read-only access available</small>
          </InsetRow>
        )}
      </Inset>

      <SectionLabel>Teams on this server</SectionLabel>
      <Inset className="settings-inset middle wide">
        {locked ? (
          <InsetRow label="Teams">
            <span className="stopped">
              {groups.length
                ? `${groups.map((store) => store.name).join(', ')} — locked`
                : 'Hidden while locked'}
            </span>
          </InsetRow>
        ) : groups.length ? (
          groups.map((store) => {
            const description = storeDescription(agentSnapshot, store);
            const abnormal =
              storeAttentionState(agentSnapshot, store) !== 'normal';
            return (
              <InsetRow
                key={store.id}
                className="devrow"
                action={
                  <>
                    {abnormal ? <Chip tone="warn">{description}</Chip> : null}
                    <Button
                      size="sm"
                      aria-label={`Open ${store.name}`}
                      onClick={() => onOpenGroup(store)}
                    >
                      Open
                    </Button>
                  </>
                }
              >
                <GroupMark store={store} size="sm" />
                <span className="t">
                  <b>{store.name}</b>
                  <small>
                    {/* This page is already about one server, so the caption
                        does not repeat it. */}
                    {teamCaption(agentSnapshot, store, { server: false })}
                    {abnormal ? '' : ` · ${description}`}
                  </small>
                </span>
              </InsetRow>
            );
          })
        ) : inventoryReadiness(agentSnapshot, 'teams', server.profileName) !==
          'ready' ? (
          <CollectionStatus
            state={inventoryReadiness(
              agentSnapshot,
              'teams',
              server.profileName,
            )}
            label="teams"
          />
        ) : (
          <InsetRow label="Teams">No teams on this server</InsetRow>
        )}
      </Inset>

      <SectionLabel>Identity and trust</SectionLabel>
      {hasHost && host ? (
        <Inset className="settings-inset middle">
          <InsetRow label="Internal ID">{server.profileName}</InsetRow>
          <InsetRow label="Address">
            {status?.configuredEndpoint ?? server.configuredEndpoint}
          </InsetRow>
          <InsetRow
            label="Host ID"
            action={
              <Button
                size="sm"
                icon="copy"
                disabled={blocked}
                onClick={() => onCopy(host.hostId)}
              >
                Copy
              </Button>
            }
          >
            <span className="hostid" title={host.hostId}>
              <code>{shortId(host.hostId, 8)}</code>
            </span>
          </InsetRow>
          {/* `chain` and `epoch` are a length and a checkpoint number, not
              times: they are reported as the two numbers the agent sends. */}
          <InsetRow label="Audit log">
            Verified · {plural(host.chain, 'entry', 'entries')} · Checkpoint{' '}
            {host.epoch}
          </InsetRow>
        </Inset>
      ) : (
        <Inset className="settings-inset middle">
          <InsetRow label="Internal ID">{server.profileName}</InsetRow>
          <InsetRow label="Address">
            {status?.configuredEndpoint ?? server.configuredEndpoint}
          </InsetRow>
          <InsetRow label="Host ID">
            <span className="stopped">
              {state === 'unprobed'
                ? 'Discovered upon first verification'
                : state === 'pending'
                  ? 'Reading signed status…'
                  : 'Hidden while locked'}
            </span>
          </InsetRow>
        </Inset>
      )}

      <SectionLabel className="danger-title">Danger zone</SectionLabel>
      <Inset className="settings-inset middle danger-box">
        <InsetRow
          className="dangerrow"
          label="Remove server and credentials"
          action={
            <Button size="sm" icon="trash" variant="danger" onClick={onRemove}>
              Remove…
            </Button>
          }
        >
          <small>
            Removes this server, its credentials, trust history, cached data,
            and unfinished operations from this device.
          </small>
        </InsetRow>
      </Inset>
    </>
  );
}
