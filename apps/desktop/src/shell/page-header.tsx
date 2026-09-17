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
  /**
   * Unrendered by this component; screens still pass it through. A header that
   * wants a second line passes `PageHeaderProps.sub`, which is drawn.
   */
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
  /**
   * A mark drawn before the title, for a page that is about one subject rather
   * than a list of them — the Account tab's own account.
   */
  mark?: ReactNode;
  /**
   * A line under the title, drawn as `.loc-copy .sub`. Distinct from
   * `HeaderParts.subtitle`, which is a plain string several screens already
   * pass and this header has never drawn: an identity line carries elements,
   * such as a chip for the local alias, so it is a node. Drawing it is opt-in,
   * which leaves every existing caller's header as it is.
   */
  sub?: ReactNode;
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
  mark,
  sub,
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
                  // The current folder is the page's title, at the size every
                  // other page draws its own; the folders above it are small.
                  <h1 className="cur">{crumb.label}</h1>
                )}
              </Fragment>
            ))}
          </nav>
        ) : (
          <>
            {mark}
            <div className="loc-copy">
              <h1>{title}</h1>
              {sub === undefined ? null : <div className="sub">{sub}</div>}
            </div>
          </>
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
