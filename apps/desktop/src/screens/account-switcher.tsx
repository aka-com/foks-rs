import { localAliasOf } from '../model';
/**
 * The account switcher Account, Devices and Settings' card credentials share.
 *
 * Each of them acts on one account at a time, and the address carries that
 * account's exact StoreRef, so the switcher navigates rather than holding
 * state of its own. An account whose access has stopped stays listed, dimmed,
 * with the reason on the row.
 */

import type { ReactNode } from 'react';
import { AccountMark } from '../components/account-mark';
import {
  accountStopped,
  serverDisplayLabelForStore,
  usernameOf,
} from '../model';
import type { AccountStore, AgentSnapshot } from '../model';

export interface AccountSwitcherProps {
  snapshot: AgentSnapshot;
  stores: readonly AccountStore[];
  selected?: AccountStore;
  onSwitch: (store: AccountStore) => void;
  /**
   * The id of the SectionLabel above the switcher. The group is named by the
   * label the reader sees, rather than by a second name only a screen reader
   * hears.
   */
  labelledBy: string;
}

/** One button per account: username, alias, server, and its stopped dot. */
export function AccountSwitcher({
  snapshot,
  stores,
  selected,
  onSwitch,
  labelledBy,
}: AccountSwitcherProps): ReactNode {
  if (!stores.length) return null;
  return (
    <div className="switch" role="group" aria-labelledby={labelledBy}>
      {stores.map((store) => {
        const { stopped, reason } = accountStopped(snapshot, store);
        const on = store.id === selected?.id;
        const name = usernameOf(snapshot, store) ?? store.account;
        return (
          <button
            key={store.id}
            type="button"
            className={['acc', on ? 'on' : '', stopped ? 'off' : '']
              .filter(Boolean)
              .join(' ')}
            aria-pressed={on}
            title={stopped ? reason : undefined}
            onClick={() => onSwitch(store)}
          >
            {stopped ? <span className="dot" aria-hidden="true" /> : null}
            <AccountMark name={name} />
            <span className="t">
              <b>{name}</b>
              <small>
                {localAliasOf(snapshot, store)} ·{' '}
                {serverDisplayLabelForStore(snapshot, store)}
              </small>
              {/* The reason is on the row, not only in its tooltip: a title
                  is not read by a pointer that never rests on the button. */}
              {stopped ? <small className="why">{reason}</small> : null}
            </span>
          </button>
        );
      })}
    </div>
  );
}
