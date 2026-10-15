/**
 * What Settings › Account and Devices show when the account an address names
 * is not in the catalog, and what Account shows when this Mac holds no
 * account at all.
 */

import type { ReactNode } from 'react';
import { Button, Notice } from '../../components';
import { localAliasOf, serverDisplayLabelForStore } from '../../model';
import type { AccountStore, AgentSnapshot } from '../../model';

/** Displayed when the account an address names is not in the catalog. */
export function UnavailableAccount({
  stores,
  snapshot,
  onSelect,
  onRefresh,
}: {
  stores: readonly AccountStore[];
  snapshot: AgentSnapshot;
  onSelect: (store: AccountStore) => void;
  onRefresh: () => void;
}): ReactNode {
  return (
    <Notice
      severity="crit"
      title="Account no longer available"
      actions={
        <>
          <Button variant="primary" onClick={onRefresh}>
            Refresh
          </Button>
          {stores.map((store) => (
            <Button key={store.id} onClick={() => onSelect(store)}>
              {localAliasOf(snapshot, store)} ·{' '}
              {serverDisplayLabelForStore(snapshot, store)}
            </Button>
          ))}
        </>
      }
    >
      {/* The notice carries its actions at the right, so the sentence no
          longer points below itself for them. */}
      <p>
        This account is no longer available on this device. Refresh, or select
        another account.
      </p>
      {stores.length ? null : (
        <p>
          This device has no active account. Add and verify a server, then
          create or recover an account.
        </p>
      )}
    </Notice>
  );
}

/** Displayed when this Mac holds no account at all. */
export function NoAvailableAccount({
  onConnectGoProfile,
}: {
  onConnectGoProfile: () => void;
}): ReactNode {
  return (
    <Notice
      title="No available account on this device"
      actions={
        <Button variant="primary" onClick={onConnectGoProfile}>
          Connect existing account
        </Button>
      }
    >
      <p>
        Add and verify a server, then create or recover an account, or connect
        an existing FOKS account on your device.
      </p>
    </Notice>
  );
}
