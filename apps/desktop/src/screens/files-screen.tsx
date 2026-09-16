/**
 * The Files tab's roots page: All items, then the vaults, groups and shares on
 * this Mac.
 *
 * The rows carry the data the rail used to carry — a vault's mark and hue, a
 * group's own mark, the description as a caption when it is not the normal
 * state, and the issue dot and dimming that go with it.
 */

import type { ReactNode } from 'react';
import { Chip, Icon, SectionLabel } from '../components';
import {
  accountSubtitle,
  teamCaption,
  storeAttentionState,
  storeDescription,
  storeDescriptionState,
  storeHues,
  storeNavigationOrder,
} from '../model';
import type { AgentSnapshot, Store } from '../model';
import type { Location } from '../location';
import { PageHeader } from '../shell/page-header';
import { GroupMark } from './group-mark';

export interface FilesScreenProps {
  snapshot: AgentSnapshot;
  onNavigate: (location: Location) => void;
}

/** One root row: mark, name, one line of description, and the issue dot. */
function StoreRow({
  snapshot,
  store,
  hue,
  onOpen,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  /** The vault hue. A group carries its own mark, which needs none. */
  hue?: string;
  onOpen: () => void;
}): ReactNode {
  const description = storeDescription(snapshot, store);
  // An abnormal state is a chip at the end of the row, never a caption: the
  // caption says what the store is, the chip says what is wrong with it. A
  // roster the agent could not read is one of those states, and only
  // `storeAttentionState` knows it; the store itself is still reachable, so
  // the row is not dimmed for it.
  const abnormal = storeAttentionState(snapshot, store) !== 'normal';
  const available = storeDescriptionState(snapshot, store) === 'normal';
  const caption =
    store.kind === 'account'
      ? `Vault · ${accountSubtitle(snapshot, store)}`
      : teamCaption(snapshot, store);
  return (
    <button
      type="button"
      className={available ? 'row' : 'row off'}
      title={description || undefined}
      onClick={onOpen}
    >
      {store.kind === 'account' ? (
        <span className="kic" style={{ background: hue, color: '#fff' }}>
          <Icon name="vault" />
        </span>
      ) : (
        // One group, one mark: the row here, the Teams row and the group's own
        // page draw the same initial over the same colour.
        <GroupMark store={store} size="sm" />
      )}
      <span className="name">
        <span className="tt">
          <span>{store.name}</span>
        </span>
        <small>{caption}</small>
      </span>
      <span className="tail">
        {abnormal ? <Chip tone="warn">{description}</Chip> : null}
        {/* The row opens the store, and says so at its end. */}
        <span className="go" aria-hidden="true">
          <Icon name="chev" />
        </span>
      </span>
    </button>
  );
}

export function FilesScreen({
  snapshot,
  onNavigate,
}: FilesScreenProps): ReactNode {
  const stores = storeNavigationOrder(snapshot);
  const hues = storeHues(stores);
  const vaults = stores.filter((store) => store.kind === 'account');
  const groups = stores.filter(
    (store) => store.kind === 'team' && store.team_kind === 'named',
  );
  const shares = stores.filter(
    (store) => store.kind === 'team' && store.team_kind === 'adhoc',
  );
  const rows = (list: readonly Store[]): ReactNode =>
    list.map((store) => (
      <StoreRow
        key={store.id}
        snapshot={snapshot}
        store={store}
        hue={store.kind === 'account' ? hues.get(store.id) : undefined}
        onOpen={() => onNavigate({ kind: 'store', ref: store.id })}
      />
    ));
  return (
    <>
      <PageHeader
        title="Files"
        subtitle="Vaults, groups and shares on this device"
      />
      <div className="body nav-rows">
        <div className="list-window">
          <div className="virtual-rows">
            <button
              type="button"
              className="row"
              onClick={() => onNavigate({ kind: 'all' })}
            >
              <span className="kic Store">
                <Icon name="grid" />
              </span>
              <span className="name">
                <span className="tt">
                  <span>All items</span>
                </span>
                <small>Every store on this device</small>
              </span>
              <span className="tail">
                <span className="go" aria-hidden="true">
                  <Icon name="chev" />
                </span>
              </span>
            </button>
            <SectionLabel>Vaults</SectionLabel>
            {vaults.length ? rows(vaults) : <p className="fn">No vaults yet</p>}
            <SectionLabel>Groups</SectionLabel>
            {groups.length ? rows(groups) : <p className="fn">No groups yet</p>}
            {shares.length ? (
              <>
                <SectionLabel>Shares</SectionLabel>
                {rows(shares)}
              </>
            ) : null}
          </div>
        </div>
      </div>
    </>
  );
}
