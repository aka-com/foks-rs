/**
 * Alerts shows current warnings and blocked operations.
 *
 * `notesNow` decides what is on it: the lapsed-lease entry is a consequence
 * of the lease world, not a standing fact, so it appears only while that
 * world is lapsed. Catalog failures are retryable directly from here because
 * there is no single other screen that owns the catalog load.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button } from '../components';
import { PageHeader } from '../shell/page-header';
import { notesNow } from '../model';
import type { Notification, World } from '../model';

const ACTIONS_ARE_LATER =
  'Open the screen that owns this action: Servers or Groups.';

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
      <PageHeader title="Alerts" subtitle="" />
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
                <Button
                  disabled={!canRetry(note) || busy.has(note.id)}
                  title={
                    canRetry(note)
                      ? 'Try the catalog load again'
                      : ACTIONS_ARE_LATER
                  }
                  onClick={() => retry(note)}
                >
                  {note.action}
                </Button>
              </div>
            ))}
          </div>
        ) : (
          <p className="hint">Nothing needs attention on this Mac.</p>
        )}
      </div>
    </>
  );
}
