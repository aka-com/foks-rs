/**
 * Displays active warnings and blocked operations.
 *
 * Catalog failures can be retried directly from this screen.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Chip, Icon } from '../components';
import { PageHeader } from '../shell/page-header';
import { notesNow } from '../model';
import type { Notification, World } from '../model';

const ACTION_UNAVAILABLE =
  'Resolve this alert in Server Settings or Group Settings.';

export interface AlertsScreenProps {
  world: World;
  onRefreshWorld: () => Promise<World>;
  onError: (error: unknown) => void;
}

function canRetry(note: Notification): boolean {
  return note.action === 'Retry' && note.id.startsWith('catalog-');
}

export function AlertsScreen({
  world,
  onRefreshWorld,
  onError,
}: AlertsScreenProps): ReactNode {
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const notes = notesNow(world);

  const retry = (note: Notification): void => {
    if (!canRetry(note) || busy.has(note.id)) return;
    setBusy((current) => new Set([...current, note.id]));
    onRefreshWorld()
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
    <>
      <PageHeader title="Alerts" subtitle="System alerts and pending actions" />
      <div className="body">
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
          <div className="empty">
            <span className="big">
              <Icon name="bell" />
            </span>
            <h2>No alerts to review</h2>
            <p>All accounts and connections are operating normally.</p>
          </div>
        )}
      </div>
    </>
  );
}
