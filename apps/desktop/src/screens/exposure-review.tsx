/**
 * The exposure review's three views: the list a removal or demotion sheet
 * shows before its write, the band a team page or Devices shows while a
 * review has unmarked entries, and the sheet that walks those entries.
 * `exposure-review.ts` holds the model and the stored records.
 */

import { useEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  Chip,
  KindIcon,
  Notice,
  SectionLabel,
  SheetDialog,
} from '../components';
import type { ChipTone } from '../components';
import {
  dismissExposure,
  entryItem,
  entryState,
  markExposure,
  openEntries,
} from '../exposure-review';
import type {
  ExposureEntry,
  ExposureMark,
  ExposureReview,
  ExposureState,
} from '../exposure-review';
import { kindOf, nameOf, plural, prefixOf, storeOf } from '../model';
import type {
  AgentSnapshot,
  CollectionReadiness,
  Item,
  StoreRef,
} from '../model';

/** The toast shown when storage refuses a review the sheet said it would keep. */
export function unstoredReview(name: string): string {
  return `The list of items ${name} could read could not be stored on this device.`;
}

/** How many items the pre-write list names before it summarizes the rest. */
export const SUMMARY_ROWS = 6;

function placeOf(snapshot: AgentSnapshot, item: Item): string {
  const store = storeOf(snapshot, item.store)?.name ?? item.store;
  const folder = prefixOf(item.path);
  return folder ? `${store} · ${folder}` : store;
}

function ItemRow({
  snapshot,
  item,
  children,
}: {
  snapshot: AgentSnapshot;
  item: Item;
  children?: ReactNode;
}): ReactNode {
  return (
    <li className="exposure-row">
      <KindIcon kind={kindOf(item) === 'Password' ? 'Password' : 'Document'} />
      <span className="t">
        <b>{nameOf(item.path)}</b>
        <small>{placeOf(snapshot, item)}</small>
      </span>
      {children}
    </li>
  );
}

/**
 * The sentence naming what a device's list leaves out, or `null` when it
 * leaves out nothing.
 */
function unlistedText(
  snapshot: AgentSnapshot,
  unlisted: readonly StoreRef[],
  teamsUnlisted: boolean,
): string | null {
  const names = unlisted.map((ref) => storeOf(snapshot, ref)?.name ?? ref);
  const last = names.pop();
  const stores = last
    ? `Items in ${names.length ? `${names.join(', ')} and ${last}` : last} could not be listed and are not included.`
    : '';
  const teams = teamsUnlisted
    ? 'The account’s teams could not be listed, so items in its teams may be missing.'
    : '';
  return [stores, teams].filter(Boolean).join(' ') || null;
}

/**
 * The items a party could read, as a removal or demotion sheet lists them
 * before its write. `heading` names the list ("Items dana could read") and
 * `empty` is the sentence drawn instead when it has no item.
 */
export function ExposureSummary({
  snapshot,
  name,
  heading,
  empty,
  items,
  readiness,
  unlisted = [],
  teamsUnlisted = false,
}: {
  snapshot: AgentSnapshot;
  /** The member's username or the device's name. */
  name: string;
  heading: string;
  empty: string;
  /** Already in {@link exposureOrder}. */
  items: readonly Item[];
  readiness: CollectionReadiness;
  /** Stores whose items could not be listed, named below the list. */
  unlisted?: readonly StoreRef[];
  /** The account's team list could not be read. */
  teamsUnlisted?: boolean;
}): ReactNode {
  const omitted = unlistedText(snapshot, unlisted, teamsUnlisted);
  if (readiness === 'loading')
    return (
      <p className="fn" role="status">
        Listing the items {name} can read…
      </p>
    );
  if (readiness === 'unavailable')
    return (
      <Notice severity="info" title="Items not listed">
        <p>
          The items {name} can read could not be listed, so no review is kept
          for this change.
        </p>
      </Notice>
    );
  if (!items.length)
    return (
      <>
        <p className="fn">{empty}</p>
        {omitted ? <p className="fn">{omitted}</p> : null}
      </>
    );
  const shown = items.slice(0, SUMMARY_ROWS);
  return (
    <>
      <SectionLabel>
        {heading}
        <span className="count"> — {items.length}</span>
      </SectionLabel>
      <ul className="exposure-rows" aria-label={heading}>
        {shown.map((item) => (
          <ItemRow
            key={`${item.store}|${item.direntId}`}
            snapshot={snapshot}
            item={item}
          />
        ))}
        {items.length > shown.length ? (
          <li className="exposure-row more">
            and {plural(items.length - shown.length, 'more item')}
          </li>
        ) : null}
      </ul>
      {omitted ? <p className="fn">{omitted}</p> : null}
      <p className="fn">
        Only items this account can list are included, so the list may be
        incomplete. Credentials {name} has read stay valid until they are
        changed where they are used. After the change, this list is kept on this
        device for review.
      </p>
    </>
  );
}

/** One band per review of `scope` that still has unmarked entries. */
export function ExposureReviewBands({
  reviews,
  onReview,
}: {
  /** The reviews of the page's team or account. */
  reviews: readonly ExposureReview[];
  onReview: (id: string) => void;
}): ReactNode {
  return reviews.map((review) => {
    const open = openEntries(review);
    if (!open) return null;
    return (
      <Band
        key={review.id}
        label={`Review items ${review.name} could read`}
        action={
          <Button size="sm" onClick={() => onReview(review.id)}>
            Review…
          </Button>
        }
      >
        {open} of {plural(review.entries.length, 'item')} {review.name} could
        read {open === 1 ? 'is' : 'are'} not yet marked rotated or not needed.
      </Band>
    );
  });
}

const STATE_CHIP: Readonly<
  Record<ExposureState, { label: string; tone: ChipTone; title?: string }>
> = {
  unchanged: {
    label: 'Unchanged',
    tone: 'warn',
    title: 'The item has the version it had when access ended.',
  },
  changed: {
    label: 'Changed since removal',
    tone: 'default',
    title:
      'The item was written after access ended. A write is not proof that the credential was rotated.',
  },
  missing: {
    label: 'Not listed',
    tone: 'default',
    title:
      'No item with this identity is listed now. It was moved, renamed or removed, or its vault is not available.',
  },
  rotated: { label: 'Rotated', tone: 'ok' },
  'not-needed': { label: 'Not needed', tone: 'default' },
};

function causeText(
  snapshot: AgentSnapshot,
  review: ExposureReview,
  date: string,
): string {
  const scope = storeOf(snapshot, review.scope)?.name ?? review.scope;
  switch (review.cause) {
    case 'removed':
      return `${review.name} was removed from ${scope} on ${date}.`;
    case 'lowered':
      return `${review.name}’s role in ${scope} was lowered on ${date}.`;
    case 'device-removed':
      return `${review.name} was removed from ${scope} on ${date}.`;
  }
}

function EntryRow({
  snapshot,
  review,
  entry,
  onOpenItem,
}: {
  snapshot: AgentSnapshot;
  review: ExposureReview;
  entry: ExposureEntry;
  onOpenItem?: (item: Item) => void;
}): ReactNode {
  const item = entryItem(snapshot, entry);
  const state = entryState(snapshot, entry);
  const chip =
    state === 'changed' && review.cause === 'lowered'
      ? { ...STATE_CHIP.changed, label: 'Changed since role change' }
      : STATE_CHIP[state];
  const label = item ? nameOf(item.path) : 'Item not listed';
  // Marking replaces the row's buttons, so focus would fall to the document;
  // it moves to the row's first new button instead, so a keyboard user keeps
  // their place in the list.
  const actionsRef = useRef<HTMLSpanElement>(null);
  const refocus = useRef(false);
  const mark = (value: ExposureMark | null): void => {
    refocus.current = true;
    markExposure(review.id, entry, value);
  };
  useEffect(() => {
    if (!refocus.current) return;
    refocus.current = false;
    actionsRef.current?.querySelector<HTMLButtonElement>('button')?.focus();
  }, [entry.mark]);
  const actions = (
    <span className="exposure-actions" ref={actionsRef}>
      <Chip tone={chip.tone} title={chip.title}>
        {chip.label}
      </Chip>
      {entry.mark ? (
        <Button
          size="sm"
          aria-label={`Clear the ${chip.label} mark on ${label}`}
          onClick={() => mark(null)}
        >
          Undo
        </Button>
      ) : (
        <>
          {item && onOpenItem ? (
            <Button
              size="sm"
              aria-label={`Open ${label}`}
              onClick={() => onOpenItem(item)}
            >
              Open
            </Button>
          ) : null}
          <Button
            size="sm"
            aria-label={`Mark ${label} rotated`}
            onClick={() => mark('rotated')}
          >
            Rotated
          </Button>
          <Button
            size="sm"
            aria-label={`Mark ${label} not needed`}
            onClick={() => mark('not-needed')}
          >
            Not needed
          </Button>
        </>
      )}
    </span>
  );
  if (item)
    return (
      <ItemRow snapshot={snapshot} item={item}>
        {actions}
      </ItemRow>
    );
  return (
    <li className="exposure-row">
      <span className="t">
        <b>{label}</b>
        <small>{storeOf(snapshot, entry.store)?.name ?? entry.store}</small>
      </span>
      {actions}
    </li>
  );
}

/**
 * The entries of one review, Passwords first, each with its state and the
 * marks the user can give it. Done, or closing the sheet any other way,
 * removes a review with no unmarked entry; Clear review removes it whatever
 * its entries say.
 */
export function ExposureReviewSheet({
  snapshot,
  review,
  onOpenItem,
  onClose,
}: {
  snapshot: AgentSnapshot;
  review: ExposureReview;
  /** Opens a listed item in Files, where Edit can generate a new password. */
  onOpenItem?: (item: Item) => void;
  onClose: () => void;
}): ReactNode {
  const date = new Date(review.at).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
  const groups: Record<
    'Passwords' | 'Documents' | 'Not listed',
    ExposureEntry[]
  > = { Passwords: [], Documents: [], 'Not listed': [] };
  for (const entry of review.entries) {
    const item = entryItem(snapshot, entry);
    groups[
      !item
        ? 'Not listed'
        : kindOf(item) === 'Password'
          ? 'Passwords'
          : 'Documents'
    ].push(entry);
  }
  const byPath = (left: ExposureEntry, right: ExposureEntry) =>
    (entryItem(snapshot, left)?.path ?? '').localeCompare(
      entryItem(snapshot, right)?.path ?? '',
    );
  const open = openEntries(review);
  // Closing by any route drops a review whose entries are all marked, as
  // Done does; otherwise it would stay stored with no band leading to it.
  const close = (): void => {
    if (!open) dismissExposure(review.id);
    onClose();
  };
  return (
    <SheetDialog
      onClose={close}
      width="wide"
      title={`Review items ${review.name} could read`}
      footer={
        <>
          <Button
            onClick={() => {
              dismissExposure(review.id);
              onClose();
            }}
          >
            Clear review
          </Button>
          <Button variant="primary" onClick={close}>
            Done
          </Button>
        </>
      }
    >
      <p>
        {causeText(snapshot, review, date)} Each item below was readable before
        the change. Change each credential where it is used, then mark it
        Rotated, or Not needed if it grants nothing outside this vault.
      </p>
      <p className="fn">
        A changed item was written after the change; that does not show that the
        credential was rotated.
      </p>
      {(['Passwords', 'Documents', 'Not listed'] as const).map((group) =>
        groups[group].length ? (
          <section key={group}>
            <SectionLabel>
              {group}
              <span className="count"> — {groups[group].length}</span>
            </SectionLabel>
            <ul className="exposure-rows" aria-label={group}>
              {groups[group].sort(byPath).map((entry) => (
                <EntryRow
                  key={`${entry.store}|${entry.dirent}`}
                  snapshot={snapshot}
                  review={review}
                  entry={entry}
                  onOpenItem={onOpenItem}
                />
              ))}
            </ul>
          </section>
        ) : null,
      )}
    </SheetDialog>
  );
}
