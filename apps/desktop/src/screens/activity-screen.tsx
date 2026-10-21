/**
 * Activity: a placeholder for the page that will gather what needs the
 * reader across accounts — membership requests, unfinished operations and
 * server trust notices. Until it exists, the page points at the places that
 * hold those today rather than drawing an empty list that would read as
 * "nothing needs attention".
 */

import type { ReactNode } from 'react';
import type { Location } from '../location';
import { Button, EmptyState } from '../components';

export function ActivityScreen({
  onNavigate,
}: {
  onNavigate: (location: Location) => void;
}): ReactNode {
  return (
    <div className="body page-body">
      <EmptyState
        icon="activity"
        title="Activity is not available yet"
        body="Requests to join a team are on the Teams page. Server status and notices are in Settings › Account."
      >
        <Button onClick={() => onNavigate({ kind: 'teams' })}>
          Open Teams
        </Button>
        <Button
          onClick={() => onNavigate({ kind: 'settings', section: 'account' })}
        >
          Open Settings
        </Button>
      </EmptyState>
    </div>
  );
}
