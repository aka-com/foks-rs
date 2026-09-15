/**
 * Page header displaying title, subtitle, and search input.
 *
 * Search input state is controlled by the active location. When searching is
 * unsupported for the current view, omitting the query handlers hides the input.
 */

import type { ReactNode } from 'react';
import { SearchField } from '../components';
import { storeHeadingDescription, storeOf } from '../model';
import type { AgentSnapshot } from '../model';
import type { Location } from '../location';

export interface HeaderParts {
  title: string;
  subtitle: string;
  tail?: ReactNode;
}

/** The title, subtitle, and optional trailing element for an item list. */
export function headerFor(
  snapshot: AgentSnapshot,
  location: Location,
): HeaderParts {
  if (location.kind === 'all') {
    return { title: 'All items', subtitle: '' };
  }
  if (location.kind !== 'store') {
    return { title: 'FOKS', subtitle: '' };
  }
  const store = storeOf(snapshot, location.ref);
  if (!store) return { title: 'Unknown vault', subtitle: '' };
  const description = storeHeadingDescription(snapshot, store);
  return {
    title: store.name,
    subtitle: description,
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
  tail,
  action,
  query,
  onQuery,
}: PageHeaderProps): ReactNode {
  return (
    <div className="path">
      <div className="loc">
        <div className="loc-copy">
          <h1>{title}</h1>
          {subtitle ? <small>{subtitle}</small> : null}
        </div>
      </div>
      {tail || action ? (
        <div className="header-action">
          {tail}
          {action}
        </div>
      ) : null}
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
