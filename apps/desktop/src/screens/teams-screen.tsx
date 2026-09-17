/**
 * The Teams tab: the groups and shares on this Mac, then the Groups pane that
 * creates and finds them.
 *
 * A row opens the group's settings on its People tab; the group settings page
 * returns here.
 */

import type { ReactNode } from 'react';
import { Chip, Icon } from '../components';
import {
  serverOf,
  storeDescription,
  storeDescriptionState,
  storeHues,
  storeNavigationOrder,
} from '../model';
import type { AgentSnapshot, TeamStore } from '../model';
import type { Location } from '../location';
import { SettingsScreen } from './settings-screen';
import type { SettingsScreenProps } from './settings-screen';

export interface TeamsScreenProps extends Omit<
  SettingsScreenProps,
  'variant' | 'before' | 'location'
> {
  location: Extract<Location, { kind: 'teams' }>;
}

/** The list above the Groups pane. */
function TeamRows({
  snapshot,
  onOpen,
}: {
  snapshot: AgentSnapshot;
  onOpen: (store: TeamStore) => void;
}): ReactNode {
  const stores = storeNavigationOrder(snapshot);
  const hues = storeHues(stores);
  const teams = stores.filter(
    (store): store is TeamStore => store.kind === 'team',
  );
  if (!teams.length)
    return <p className="fn">No groups or shares on this Mac yet.</p>;
  return (
    <div className="list-window">
      <div className="virtual-rows">
        {teams.map((store) => {
          const description = storeDescription(snapshot, store);
          // The abnormal state is the row's chip, not its caption.
          const state = storeDescriptionState(snapshot, store);
          const server = serverOf(snapshot, store.id);
          return (
            <button
              type="button"
              key={store.id}
              className={state === 'normal' ? 'row' : 'row off'}
              title={description || undefined}
              onClick={() => onOpen(store)}
            >
              <span
                className="kic"
                style={{ background: hues.get(store.id), color: '#fff' }}
              >
                <Icon name="people" />
              </span>
              <span className="name">
                <span className="tt">
                  <span>{store.name}</span>
                </span>
                <small>
                  {[
                    server?.name ?? store.server,
                    state === 'normal' ? description : '',
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </small>
              </span>
              <span className="tail">
                {state === 'normal' ? null : (
                  <Chip tone="warn">{description}</Chip>
                )}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function TeamsScreen({
  snapshot,
  onNavigate,
  ...rest
}: TeamsScreenProps): ReactNode {
  return (
    <SettingsScreen
      {...rest}
      snapshot={snapshot}
      onNavigate={onNavigate}
      variant="teams"
      before={
        <div className="nav-rows">
          <TeamRows
            snapshot={snapshot}
            onOpen={(store) =>
              onNavigate({
                kind: 'group-settings',
                ref: store.id,
                tab: 'people',
              })
            }
          />
        </div>
      }
    />
  );
}
