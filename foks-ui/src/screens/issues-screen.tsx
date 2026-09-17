/**
 * Issues shows current warnings and blocked operations.
 *
 * `notesNow` decides what is on it: the lapsed-lease entry is a consequence
 * of the lease world, not a standing fact, so it appears only while that
 * world is lapsed. Each card says what would change it, and every action
 * belongs to the screen that owns that operation.
 */

import type { ReactNode } from 'react';
import { Button } from '../components';
import { PageHeader } from '../shell/page-header';
import { notesNow } from '../model';
import type { World } from '../model';

const ACTIONS_ARE_LATER =
  'Open the screen that owns this action: Servers & devices or Groups.';

export function IssuesScreen({ world }: { world: World }): ReactNode {
  const notes = notesNow(world);
  return (
    <>
      <PageHeader title="Issues" subtitle="" />
      <div className="body">
        <div className="cards">
          {notes.map((note) => (
            <div className="card" key={note.id}>
              <span className={`sev ${note.severity}`} />
              <div>
                <h3>{note.title}</h3>
                <p>{note.detail}</p>
              </div>
              <Button disabled title={ACTIONS_ARE_LATER}>
                {note.action}
              </Button>
            </div>
          ))}
        </div>
      </div>
    </>
  );
}
