/**
 * The panes this phase does not build.
 *
 * Join or create a group, Servers, Settings and the first run are
 * whole screens. A stub of them would be a screen someone has to delete, so
 * this identifies unavailable locations without inventing their behavior.
 */

import type { ReactNode } from 'react';
import { PageHeader } from '../shell/page-header';
import type { Location } from '../location';

interface Pane {
  title: string;
  subtitle: string;
  body: string;
}

const PANES: Readonly<Record<string, Pane>> = {
  join: {
    title: 'Join or create a group',
    subtitle: 'Not available in this build',
    body: 'An admin adds your username on the server. FOKS does not use invite links. Open Groups to manage rosters, roles, and Federation.',
  },
  servers: {
    title: 'Servers',
    subtitle: 'Not available in this build',
    body: 'Server rows with their own Check, the pinned Host ID, the check-in and how long it has left, Add a server, Forget, the typed-confirmation reset and pairing are all specified there.',
  },
  settings: {
    title: 'Settings',
    subtitle: 'Not available in this build',
    body: 'Your accounts on this Mac, Recovery devices, Security keys grouped into Everyday / Recovery / Danger, the backup phrase, Agent status and About are all specified there.',
  },
  'first-run': {
    title: 'Setting up',
    subtitle: 'Not available in this build',
    body: 'First run is a mode, not a window: the same shell with the sidebar in its step-list form. Walking it again does not undo what is already set up — each step says what it finds and what it would change.',
  },
};

export function PlaceholderScreen({
  location,
}: {
  location: Location;
}): ReactNode {
  const pane = PANES[location.kind];
  if (!pane) return null;
  return (
    <>
      <PageHeader title={pane.title} subtitle={pane.subtitle} />
      <div className="toolbar" />
      <div className="body">
        <div className="plain">
          <h2>{pane.title} has its own screen, and it is not this one</h2>
          <p>{pane.body}</p>
          <p className="hint">This surface is not available in this build.</p>
        </div>
      </div>
    </>
  );
}
