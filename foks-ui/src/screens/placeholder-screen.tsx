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
      <div className="body">
        <div className="plain">
          <h2>{pane.title} is coming soon</h2>
          <p>{pane.body}</p>
          <p className="hint">This surface is not available in this build.</p>
        </div>
      </div>
    </>
  );
}
