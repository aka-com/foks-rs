/**
 * The badge a store carries in the Files tree, list rows, title bar and the
 * details panel's Location row.
 */

import type { ReactNode } from 'react';
import { initials, usernameOf } from '../model';
import type { AgentSnapshot, Store } from '../model';
import { AccountMark } from './account-mark';

/**
 * A team store draws its initials over its hue from `storeHues`; an account
 * store draws its `AccountMark`. The caller passes the hue so a list computes
 * the hue map once rather than once per row.
 */
export function StoreMark({
  snapshot,
  store,
  hue,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  hue: string | undefined;
}): ReactNode {
  return store.kind === 'team' ? (
    <span className="av team" style={{ background: hue }} aria-hidden="true">
      {initials(store.name)}
    </span>
  ) : (
    <AccountMark name={usernameOf(snapshot, store) ?? store.account} />
  );
}
