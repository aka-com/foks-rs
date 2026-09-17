/**
 * The page header — `shell.js`'s `pageHeader()` and `headerParts()`.
 *
 * Title, subtitle, the avatar stack on a group, the Manage chip, and the
 * search field with its ⌘K hint. Store descriptions come from the same model
 * function as the sidebar, so the two surfaces cannot disagree.
 */

import type { ReactNode } from 'react';
import { Button, SearchField, Stack } from '../components';
import {
  partiesOf,
  storeDescription,
  storeDescriptionState,
  storeOf,
} from '../model';
import type { World } from '../model';
import type { Location } from '../location';

export interface HeaderParts {
  title: string;
  subtitle: string;
  lead?: ReactNode;
  tail?: ReactNode;
}

/** The title, subtitle, and optional leading or trailing elements for an item list. */
export function headerFor(world: World, location: Location, onManage?: () => void): HeaderParts {
  if (location.kind === 'all') {
    return { title: 'All items', subtitle: '' };
  }
  if (location.kind !== 'store') {
    return { title: 'FOKS', subtitle: '' };
  }
  const store = storeOf(world, location.ref);
  if (!store) return { title: 'Unknown vault', subtitle: '' };
  const description = storeDescription(world, store);
  const descriptionState = storeDescriptionState(world, store);
  if (store.kind === 'account') {
    return {
      title: store.name,
      subtitle: description,
    };
  }
  const parties = partiesOf(world, store.id);
  return {
    title: store.name,
    subtitle: description,
    lead: <Stack parties={parties} size="lg" />,
    tail:
      descriptionState === 'normal' ? (
        <Button
          size="sm"
          disabled={!onManage || store.team_kind === 'adhoc'}
          onClick={onManage}
          title={store.team_kind === 'adhoc'
            ? 'Roster management is unavailable for an ad-hoc group'
            : `People and roles in ${store.name}`}
        >
          Manage
        </Button>
      ) : undefined,
  };
}

/** "Search all items" / "Search Household" / "Search" when that is too long. */
export function searchPlaceholder(title: string): string {
  if (title === 'All items') return 'Search all items';
  const full = `Search ${title}`;
  return full.length > 18 ? 'Search' : full;
}

export interface PageHeaderProps extends HeaderParts {
  /** A page-level action aligned at the far right of the header. */
  action?: ReactNode;
  /** Omitted on a pane that has nothing to search. */
  query?: string;
  onQuery?: (query: string) => void;
}

export function PageHeader({
  title,
  subtitle,
  lead,
  tail,
  action,
  query,
  onQuery,
}: PageHeaderProps): ReactNode {
  return (
    <div className="path">
      <div className={`loc${lead || tail ? '' : ' text-only'}`}>
        {lead}
        <h1>{title}</h1>
        {subtitle ? <small>{subtitle}</small> : null}
        {tail}
      </div>
      {action ? <div className="header-action">{action}</div> : null}
      {query !== undefined && onQuery ? (
        <SearchField
          value={query}
          onChange={onQuery}
          placeholder={searchPlaceholder(title)}
        />
      ) : null}
    </div>
  );
}
