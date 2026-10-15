/**
 * A row of the Files sidebar tree: All items, a vault, one of its folders,
 * Trash, or the invitation a section draws when it has no rows.
 */

import type { ReactNode } from 'react';
import { Icon } from '../../components';
import type { Store } from '../../model';
import type { FolderNode } from '../../model/scope';

/** A vault and its folders, as the sidebar tree draws them. */
export interface StoreTree {
  store: Store;
  root: FolderNode;
}

export function TreeRow({
  depth,
  active,
  root,
  quiet,
  icon,
  mark,
  name,
  count,
  tag,
  open,
  expandable,
  action,
  onSelect,
  onToggle,
  storeId,
  path,
  drop,
  tree,
}: {
  storeId?: string;
  path?: string;
  /** Rows dragged over this folder: whether they can be dropped here. */
  drop?: 'ok' | 'no';
  /**
   * The row's place in a folder tree, where it is a treeitem: the whole row
   * is the focus target, and the twist and its own action are pointer
   * shortcuts for the arrow keys and the context menu. A row without one is
   * a single button outside any tree.
   */
  tree?: { key: string; level: number; tabIndex: 0 | -1 };
  depth: number;
  active: boolean;
  root?: boolean;
  /** The muted invitation row a section draws in place of no rows at all. */
  quiet?: boolean;
  icon: 'folder' | 'vault' | 'grid' | 'plus' | 'trash';
  /** Drawn in place of `icon` when the row names a store. */
  mark?: ReactNode;
  name: string;
  /** Omitted, not drawn as 0, when there is nothing to count. */
  count?: number;
  /** Replaces the count when the row has a condition to state instead. */
  tag?: ReactNode;
  open: boolean;
  expandable: boolean;
  /** A control that belongs to the row, drawn after its name. */
  action?: ReactNode;
  onSelect: () => void;
  onToggle: () => void;
}): ReactNode {
  const className = [
    'fn',
    active ? 'on' : '',
    root ? 'root' : '',
    quiet ? 'quiet' : '',
    drop ? `drop-${drop}` : '',
  ]
    .filter(Boolean)
    .join(' ');
  const style = { '--d': depth } as React.CSSProperties;
  const label = (
    <>
      {mark ?? <Icon name={icon} />}
      <span className="nm">{name}</span>
    </>
  );
  const trailing = (
    <>
      {action}
      {tag ?? (count === undefined ? null : <span className="c">{count}</span>)}
    </>
  );
  if (!tree)
    return (
      <div className={className} style={style}>
        <span className="twist none" aria-hidden="true" />
        <button
          type="button"
          className="fselect"
          aria-current={active ? 'location' : undefined}
          onClick={onSelect}
        >
          {label}
        </button>
        {trailing}
      </div>
    );
  return (
    <div
      className={className}
      data-folder-store={storeId}
      data-folder-path={path}
      data-tree-key={tree.key}
      style={style}
      role="treeitem"
      aria-level={tree.level}
      aria-expanded={expandable ? open : undefined}
      aria-selected={active}
      aria-haspopup={storeId ? 'menu' : undefined}
      tabIndex={tree.tabIndex}
      onClick={onSelect}
    >
      {/* Every row keeps the twist cell so names align down the column; a row
          with nothing to disclose leaves it empty. */}
      <span
        className={expandable ? (open ? 'twist open' : 'twist') : 'twist none'}
        aria-hidden="true"
        onClick={
          expandable
            ? (event) => {
                event.stopPropagation();
                onToggle();
              }
            : undefined
        }
      >
        {expandable ? <Icon name="chevronDown" /> : null}
      </span>
      <span className="fselect">{label}</span>
      {trailing}
    </div>
  );
}
