/**
 * Placeholder screen for views that are not yet implemented.
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
    title: 'Join or create a team',
    subtitle: 'Coming soon',
    body: 'To join an existing team, ask an administrator to add your username on the server. To manage your current teams, rosters, and roles, go to Teams.',
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
          <h2>Team management coming soon</h2>
          <p>
            Joining and creating teams will be available in an upcoming release.
          </p>
        </div>
      </div>
    </>
  );
}
