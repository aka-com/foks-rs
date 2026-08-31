/**
 * The item page's toolbar — `01-vault.html`'s `toolbar` string, as controls.
 *
 * New (a split button over the kind menu) · the kind filter · Sort · list or
 * cards · the details toggle. The filter, the sort, the view and the panel
 * are navigation state, so each one is a deep link and survives a reload.
 *
 * New offers the four product kinds. The sheet binds an account create to its
 * fixed Owner roles or asks for both group roles explicitly.
 */

import type { ReactNode } from 'react';
import {
  Button,
  Icon,
  KindIcon,
  MenuButton,
  SegmentedControl,
  SplitButton,
} from '../components';
import { KINDS, KIND_LIST } from '../model';
import type { KindFilter, SortKey, ViewMode } from '../location';

const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  name: 'Name',
  kind: 'Kind',
  group: 'Group',
  version: 'Version',
};

/** The shortcuts the design prints beside the four kinds. */
const NEW_KEYS: Readonly<Record<string, string>> = {
  Password: '⌘N',
  Resource: '⇧⌘N',
  File: '⌘U',
  Link: '⌘L',
};

export interface ToolbarProps {
  onNew: (kind: Exclude<KindFilter, 'All'>) => void;
  kind: KindFilter;
  onKind: (kind: KindFilter) => void;
  sort: SortKey;
  onSort: (sort: SortKey) => void;
  view: ViewMode;
  onView: (view: ViewMode) => void;
  details: boolean;
  onDetails: (open: boolean) => void;
}

export function Toolbar({
  onNew,
  kind,
  onKind,
  sort,
  onSort,
  view,
  onView,
  details,
  onDetails,
}: ToolbarProps): ReactNode {
  return (
    <div className="toolbar">
      <SplitButton label="New" icon="plus" menuLabel="What to create">
        {(close) => (
          <>
            <div className="cap">
              Saved into one vault or group; the sheet asks which.
            </div>
            {KIND_LIST.map((name) => (
              <button
                key={name}
                type="button"
                role="menuitem"
                onClick={() => {
                  close();
                  onNew(name);
                }}
              >
                <KindIcon kind={name} />
                {name}
                <kbd>{NEW_KEYS[name]}</kbd>
              </button>
            ))}
          </>
        )}
      </SplitButton>
      <SegmentedControl<KindFilter>
        label="Which kinds to list"
        value={kind}
        onChange={onKind}
        items={[
          { id: 'All', label: 'All' },
          ...KIND_LIST.map((name) => ({
            id: name,
            label: KINDS[name].plural,
            title: KINDS[name].blurb,
          })),
        ]}
      />
      <span className="spacer" />
      <MenuButton label="Sort" menuLabel="Sort the list by" align="end">
        {(close) => (
          <>
            {(Object.keys(SORT_LABELS) as SortKey[]).map((key) => (
              <button
                key={key}
                type="button"
                role="menuitem"
                className={sort === key ? 'on' : ''}
                aria-checked={sort === key}
                onClick={() => {
                  onSort(key);
                  close();
                }}
              >
                {sort === key ? <Icon name="check" /> : <span className="ic" />}
                {SORT_LABELS[key]}
              </button>
            ))}
          </>
        )}
      </MenuButton>
      <SegmentedControl<ViewMode>
        label="How to show the items"
        variant="icon"
        value={view}
        onChange={onView}
        items={[
          { id: 'list', icon: 'list', title: 'List' },
          { id: 'grid', icon: 'grid', title: 'Cards' },
        ]}
      />
      <Button
        variant="quiet"
        icon="info"
        on={details}
        title="Details"
        aria-label="Details"
        onClick={() => {
          onDetails(!details);
        }}
      />
    </div>
  );
}
