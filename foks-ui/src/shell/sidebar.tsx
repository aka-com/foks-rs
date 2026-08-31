/**
 * The sidebar, as `shell.js`'s `sidebar()` draws it.
 *
 * All items · VAULTS · GROUPS, then the footer. Three readings are
 * computed, never typed: an account names its server, an active group carries
 * its roster count, and an unavailable or inactive store says "Connection
 * error". The Issues badge is absent at zero (`Badge`), not a "0".
 *
 * Rows are buttons in fixture order, which is the order the mock lists them:
 * `sidebar()` walks `FX.stores`; `storeDisplayOrder` is for the item page's
 * sections, not this list.
 */

import type { ReactNode } from 'react';
import { Badge, Icon, SectionLabel, Stack } from '../components';
import {
  partiesOf,
  storeDescription,
  storeDescriptionState,
} from '../model';
import type { Store, World } from '../model';
import type { Location } from '../location';

interface NavRowProps {
  active: boolean;
  glyph: ReactNode;
  name: string;
  caption?: string;
  /** The amber issue dot. */
  dot?: boolean;
  /** Dimmed: this row is not reachable in the world as it stands. */
  dimmed?: boolean;
  title?: string;
  tail?: ReactNode;
  onSelect: () => void;
}

function NavRow({
  active,
  glyph,
  name,
  caption,
  dot = false,
  dimmed = false,
  title,
  tail,
  onSelect,
}: NavRowProps): ReactNode {
  const className = ['nav', active ? 'on' : '', dimmed ? 'off' : '']
    .filter(Boolean)
    .join(' ');
  return (
    <button
      type="button"
      className={className}
      title={title}
      aria-current={active ? 'page' : undefined}
      onClick={onSelect}
    >
      {glyph}
      <span className="t">
        {name}
        {caption ? <small>{caption}</small> : null}
      </span>
      {dot ? <span className="dot" title="Has issues" /> : null}
      {tail}
    </button>
  );
}

export interface SidebarProps {
  world: World;
  location: Location;
  /** How many notifications apply to the world as it stands. */
  issues: number;
  onNavigate: (location: Location) => void;
}

export function Sidebar({
  world,
  location,
  issues,
  onNavigate,
}: SidebarProps): ReactNode {
  const storeRow = (store: Store): ReactNode => {
    const connectionError = storeDescriptionState(world, store) !== 'normal';
    return (
      <NavRow
        key={store.id}
        active={location.kind === 'store' && location.ref === store.id}
        glyph={
          store.kind === 'account' ? (
            <Icon name="vault" />
          ) : (
            <Stack parties={partiesOf(world, store.id)} />
          )
        }
        name={store.name}
        caption={storeDescription(world, store)}
        dot={connectionError}
        dimmed={connectionError}
        onSelect={() => {
          onNavigate({ kind: 'store', ref: store.id });
        }}
      />
    );
  };

  return (
    <nav className="side" aria-label="Places">
      <NavRow
        active={location.kind === 'all'}
        glyph={<Icon name="grid" />}
        name="All items"
        onSelect={() => {
          onNavigate({ kind: 'all' });
        }}
      />
      <SectionLabel as="side">Vaults</SectionLabel>
      {world.stores.filter((store) => store.kind === 'account').map(storeRow)}
      <SectionLabel as="side">Groups</SectionLabel>
      {world.stores.filter((store) => store.kind === 'team').map(storeRow)}
      <div className="foot">
        <NavRow
          active={location.kind === 'join' || location.kind === 'groups' || location.kind === 'group-admin'}
          glyph={<Icon name="plus" />}
          name="Join or create a group"
          onSelect={() => {
            onNavigate({ kind: 'groups' });
          }}
        />
        <NavRow
          active={location.kind === 'servers'}
          glyph={<Icon name="server" />}
          name="Servers & devices"
          onSelect={() => {
            onNavigate({ kind: 'servers' });
          }}
        />
        <NavRow
          active={location.kind === 'issues'}
          glyph={<Icon name="bell" />}
          name="Issues"
          tail={<Badge count={issues} label="Open issues" />}
          onSelect={() => {
            onNavigate({ kind: 'issues' });
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
        <NavRow
          active={location.kind === 'first-run'}
          glyph={<Icon name="again" />}
          name="Set up again"
          title="Walk the first run again — nothing already set up is undone"
          onSelect={() => {
            onNavigate({ kind: 'first-run', step: 'who' });
          }}
        />
      </div>
    </nav>
  );
}
