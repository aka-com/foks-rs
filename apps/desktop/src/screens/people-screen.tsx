/**
 * The People tab: what needs attention, then the accounts on this Mac.
 *
 * `AttentionList` is the page that used to be called Alerts. It keeps its
 * catalog retry; the word "Alerts" is gone and the list is a section of People.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Chip, SectionLabel } from '../components';
import { notesNow } from '../model';
import type { Notification, AgentSnapshot } from '../model';
import type { Location } from '../location';
import { SettingsScreen } from './settings-screen';
import type { SettingsScreenProps } from './settings-screen';

const ACTION_UNAVAILABLE =
  'Resolve this in Settings › Servers, or in the group’s settings.';

export interface AttentionListProps {
  snapshot: AgentSnapshot;
  onRefreshSnapshot: () => Promise<AgentSnapshot>;
  onError: (error: unknown) => void;
}

function canRetry(note: Notification): boolean {
  return note.action === 'Retry' && note.id.startsWith('catalog-');
}

/** Active warnings and blocked operations, with the retries they allow. */
export function AttentionList({
  snapshot,
  onRefreshSnapshot,
  onError,
}: AttentionListProps): ReactNode {
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const notes = notesNow(snapshot);

  const retry = (note: Notification): void => {
    if (!canRetry(note) || busy.has(note.id)) return;
    setBusy((current) => new Set([...current, note.id]));
    onRefreshSnapshot()
      .catch(onError)
      .finally(() => {
        setBusy((current) => {
          const next = new Set(current);
          next.delete(note.id);
          return next;
        });
      });
  };

  return (
    <div className="people-attention">
      <SectionLabel>Needs attention</SectionLabel>
      {notes.length ? (
        <div className="cards">
          {notes.map((note) => (
            <div className="card" key={note.id}>
              <span className={`sev ${note.severity}`} />
              <div>
                <h3>{note.title}</h3>
                <p>{note.detail}</p>
              </div>
              {canRetry(note) ? (
                <Button
                  disabled={busy.has(note.id)}
                  title="Retry loading catalog"
                  onClick={() => retry(note)}
                >
                  {note.action}
                </Button>
              ) : note.action ? (
                <Chip tone="warn" title={ACTION_UNAVAILABLE}>
                  Action required
                </Chip>
              ) : null}
            </div>
          ))}
        </div>
      ) : (
        <p className="fn">
          Nothing needs attention. All accounts and connections are operating
          normally.
        </p>
      )}
    </div>
  );
}

export interface PeopleScreenProps extends Omit<
  SettingsScreenProps,
  'variant' | 'before' | 'location'
> {
  location: Extract<Location, { kind: 'people' }>;
}

export function PeopleScreen({
  snapshot,
  onRefreshSnapshot,
  onError,
  ...rest
}: PeopleScreenProps): ReactNode {
  return (
    <SettingsScreen
      {...rest}
      snapshot={snapshot}
      onRefreshSnapshot={onRefreshSnapshot}
      onError={onError}
      variant="people"
      before={
        <AttentionList
          snapshot={snapshot}
          onRefreshSnapshot={onRefreshSnapshot}
          onError={onError}
        />
      }
    />
  );
}
