/**
 * Page header displaying the title, actions, and search input.
 *
 * Search input state is controlled by the active location. When searching is
 * unsupported for the current view, omitting the query handlers hides the input.
 *
 * The Files browser has no plain title: its header is a breadcrumb of the
 * folder the tree has open, so it passes `crumbs` instead. The crumb trail is
 * the same markup the folders view used to draw in its own list pane, moved
 * here now that it is the page's own header rather than a pane's.
 */

import { Fragment } from 'react';
import type { ReactNode } from 'react';
import { Icon, SearchField } from '../components';

export interface HeaderParts {
  title: string;
  /** Unrendered by this component today; screens still pass it through. */
  subtitle?: string;
  tail?: ReactNode;
}

/** One segment of a breadcrumb header. The last segment has no `onClick`. */
export interface Crumb {
  label: string;
  onClick?: () => void;
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
  /** A rule under the header, for pages with no toolbar to carry one. */
  ruled?: boolean;
  /** Omitted on a pane that has nothing to search. */
  query?: string;
  onQuery?: (query: string) => void;
  /** A clickable folder path in place of the plain title. */
  crumbs?: readonly Crumb[];
}

export function PageHeader({
  title,
  crumbs,
  tail,
  action,
  ruled = false,
  query,
  onQuery,
}: PageHeaderProps): ReactNode {
  return (
    <div className={ruled ? 'path ruled' : 'path'}>
      <div className="loc">
        {crumbs && crumbs.length ? (
          <nav className="crumbs" aria-label="Current folder">
            {crumbs.map((crumb, index) => (
              // Keyed by position as well as label: a folder can share a name
              // with its own vault or an ancestor folder.
              <Fragment key={`${index}-${crumb.label}`}>
                {index > 0 ? (
                  <span className="sep">
                    <Icon name="chev" />
                  </span>
                ) : null}
                {crumb.onClick ? (
                  <button type="button" onClick={crumb.onClick}>
                    {crumb.label}
                  </button>
                ) : (
                  <span className="cur">{crumb.label}</span>
                )}
              </Fragment>
            ))}
          </nav>
        ) : (
          <div className="loc-copy">
            <h1>{title}</h1>
          </div>
        )}
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
