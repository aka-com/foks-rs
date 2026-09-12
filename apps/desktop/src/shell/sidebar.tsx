import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
/**
 * Primary sidebar navigation component.
 *
 * Renders account vaults, groups, alerts, and settings based on current agent
 * state. Control-Tab navigation follows the displayed store order.
 */

import { Fragment, useEffect, type ReactNode } from 'react';
import { Badge, Icon, SectionLabel, Stack } from '../components';
import {
  partiesOf,
  storeDescription,
  storeDescriptionState,
  storeNavigationOrder,
} from '../model';
import type { Store, World } from '../model';
import { sameLocation } from '../location';
import type { Location } from '../location';

function chatAvailable(world: World, store: Store): boolean {
  return (
    store.kind === 'team' &&
    store.team_kind === 'named' &&
    store.active !== false &&
    world.servers.find((server) => server.id === store.server)
      ?.chat_available === true
  );
}

/** Places Control-Tab walks, in sidebar order. The footer is not included. */
export function sidebarCycleLocations(world: World): Location[] {
  return [
    { kind: 'all' },
    ...storeNavigationOrder(world).flatMap((store): Location[] => [
      { kind: 'store', ref: store.id },
      ...(chatAvailable(world, store)
        ? [{ kind: 'team-chat' as const, ref: store.id }]
        : []),
    ]),
  ];
}

export function nextSidebarCycleLocation(
  world: World,
  current: Location,
  delta: 1 | -1,
): Location | null {
  const places = sidebarCycleLocations(world);
  if (!places.length) return null;
  const index = places.findIndex((place) => sameLocation(place, current));
  if (index < 0) return delta > 0 ? places[0] : places[places.length - 1];
  return places[(index + delta + places.length) % places.length];
}

export interface NavRowProps {
  active: boolean;
  glyph?: ReactNode;
  indented?: boolean;
  name: string;
  caption?: string;
  /** Dimmed: this row is unavailable in the current application state. */
  dimmed?: boolean;
  disabled?: boolean;
  title?: string;
  tail?: ReactNode;
  onSelect: () => void;
}

export function NavRow({
  active,
  glyph,
  indented = false,
  name,
  caption,
  dimmed = false,
  disabled = false,
  title,
  tail,
  onSelect,
}: NavRowProps): ReactNode {
  const className = [
    'nav',
    active ? 'on' : '',
    dimmed ? 'off' : '',
    indented ? 'indented' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <button
      type="button"
      className={className}
      title={title}
      aria-current={active ? 'page' : undefined}
      disabled={disabled}
      onClick={onSelect}
    >
      {glyph}
      <span className="t">
        {name}
        {caption ? <small>{caption}</small> : null}
      </span>
      {tail}
    </button>
  );
}

export interface SidebarProps {
  world: World;
  location: Location;
  /** Count of active notifications shown in the sidebar. */
  alerts: number;
  onNavigate: (location: Location) => void;
  /**
   * Rows between the store lists and the footer. First run puts its progress
   * there; the shell has no additional status to display at that point.
   */
  status?: ReactNode;
  /**
   * First run is already in the flow, so its last row re-enters rather than
   * navigating to the flow's first step.
   */
  onReenter?: () => void;
}

export function Sidebar({
  world,
  location,
  alerts,
  onNavigate,
  status,
  onReenter,
}: SidebarProps): ReactNode {
  const chatInbox = useSidebarInbox();
  const chatTail = (id: string) => {
    const unread = teamUnread(chatInbox.get(id));
    return unread ? (
      <span
        className="chat-unread"
        aria-label={unread.description}
        title={unread.description}
      >
        {unread.label}
      </span>
    ) : null;
  };
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (
        event.key !== 'Tab' ||
        !event.ctrlKey ||
        event.metaKey ||
        event.altKey
      )
        return;
      const next = nextSidebarCycleLocation(
        world,
        location,
        event.shiftKey ? -1 : 1,
      );
      if (!next) return;
      event.preventDefault();
      onNavigate(next);
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [location, onNavigate, world]);

  const stores = storeNavigationOrder(world);
  const vaults = stores.filter((store) => store.kind === 'account');
  const groups = stores.filter(
    (store) => store.kind === 'team' && store.team_kind === 'named',
  );
  const shares = stores.filter(
    (store) => store.kind === 'team' && store.team_kind === 'adhoc',
  );

  /**
   * Computes display state for a store. Account stores show item information,
   * while group stores include their member roster.
   */
  const storeRow = (store: Store): ReactNode => {
    const connectionError = storeDescriptionState(world, store) !== 'normal';
    return (
      <NavRow
        key={store.id}
        active={
          (location.kind === 'store' || location.kind === 'group-settings') &&
          location.ref === store.id
        }
        glyph={
          store.kind === 'account' ? (
            <Icon name="vault" />
          ) : (
            <Icon name="people" />
          )
        }
        name={store.name}
        caption={storeDescription(world, store)}
        dimmed={connectionError}
        tail={
          store.kind === 'team' ? (
            <Stack parties={partiesOf(world, store.id)} />
          ) : undefined
        }
        onSelect={() => {
          // Clicking the currently active store preserves the existing selection.
          onNavigate({ kind: 'store', ref: store.id });
        }}
      />
    );
  };

  return (
    <nav className="side" aria-label="Main Navigation">
      <NavRow
        active={location.kind === 'all'}
        glyph={<Icon name="grid" />}
        name="All items"
        onSelect={() => {
          onNavigate({ kind: 'all' });
        }}
      />
      <SectionLabel as="side">Vaults</SectionLabel>
      {vaults.length ? (
        vaults.map(storeRow)
      ) : (
        <p className="fn">No vaults yet</p>
      )}
      <SectionLabel as="side">Groups</SectionLabel>
      {groups.length ? (
        groups.map((store) => (
          <Fragment key={store.id}>
            {storeRow(store)}
            {store.kind === 'team' &&
              store.team_kind === 'named' &&
              store.active !== false && (
                <NavRow
                  active={
                    location.kind === 'team-chat' && location.ref === store.id
                  }
                  indented
                  name={`${store.name} chat`}
                  tail={
                    chatAvailable(world, store) ? chatTail(store.id) : undefined
                  }
                  caption={
                    chatAvailable(world, store)
                      ? undefined
                      : 'Chat unavailable for this server'
                  }
                  dimmed={!chatAvailable(world, store)}
                  disabled={!chatAvailable(world, store)}
                  title={
                    chatAvailable(world, store)
                      ? undefined
                      : 'The current server compatibility grant does not enable chat.'
                  }
                  onSelect={() =>
                    onNavigate({ kind: 'team-chat', ref: store.id })
                  }
                />
              )}
          </Fragment>
        ))
      ) : (
        <p className="fn">No groups yet</p>
      )}
      {shares.length ? (
        <>
          <SectionLabel as="side">Shares</SectionLabel>
          {shares.map(storeRow)}
        </>
      ) : null}
      {status}
      <div className="foot">
        <NavRow
          active={location.kind === 'alerts'}
          glyph={<Icon name="bell" />}
          name="Alerts"
          tail={<Badge count={alerts} label="open alerts" />}
          onSelect={() => {
            onNavigate({ kind: 'alerts' });
          }}
        />
        <NavRow
          active={location.kind === 'settings'}
          glyph={<Icon name="gear" />}
          name="Settings"
          onSelect={() => {
            onNavigate({ kind: 'settings' });
          }}
        />
        <div className="foot-separator" />
        <NavRow
          active={location.kind === 'first-run' && !onReenter}
          glyph={<Icon name="again" />}
          name="Set up new vault"
          title="Set up a new vault without changing existing vaults"
          onSelect={
            onReenter ??
            (() => {
              onNavigate({ kind: 'first-run', step: 'who' });
            })
          }
        />
      </div>
    </nav>
  );
}
