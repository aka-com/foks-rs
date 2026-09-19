/**
 * Isolates screen render failures from the surrounding application shell.
 *
 * A throw during a page's render otherwise unmounts the whole shell, rail and
 * topbar included, leaving nothing to navigate away with. This catches it at
 * the page: the rail stays, the page becomes a band saying what failed, with
 * a way to draw it again, and navigating to another page starts that page
 * clean because the shell keys the boundary on the location.
 */

import { Component, Fragment } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { normalizeCommandError } from '../bridge';
import { Band, Button } from '../components';
import type { Location } from '../location';

/**
 * What identifies a page for the boundary's key: the fields `sameLocation`
 * in `navigation/routes.ts` reads to tell one page from another, without
 * the conceal signal, on which the screens already remount.
 */
export function screenBoundaryKey(location: Location): string {
  return [
    location.kind,
    'ref' in location ? location.ref : '',
    'section' in location ? location.section : '',
  ].join(':');
}

interface ScreenErrorBoundaryProps {
  children: ReactNode;
}

interface ScreenErrorBoundaryState {
  /** Error caught while rendering the current screen. */
  failure: { error: unknown } | null;
  /** Incremented by Reload page to remount the screen. */
  generation: number;
}

export class ScreenErrorBoundary extends Component<
  ScreenErrorBoundaryProps,
  ScreenErrorBoundaryState
> {
  override state: ScreenErrorBoundaryState = { failure: null, generation: 0 };

  static getDerivedStateFromError(
    error: unknown,
  ): Partial<ScreenErrorBoundaryState> {
    return { failure: { error } };
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    // Display the error in the UI and retain the stack trace in the console for
    // diagnostics and tests.
    console.error('A page failed to render.', error, info.componentStack);
  }

  override render(): ReactNode {
    const { failure, generation } = this.state;
    if (failure)
      return (
        <div className="subnav-page">
          <div className="body">
            <Band
              severity="crit"
              live
              label="This page could not be shown"
              action={
                <Button
                  onClick={() =>
                    this.setState((current) => ({
                      failure: null,
                      generation: current.generation + 1,
                    }))
                  }
                >
                  Reload page
                </Button>
              }
            >
              {normalizeCommandError(failure.error).message}
            </Band>
          </div>
        </div>
      );
    return <Fragment key={generation}>{this.props.children}</Fragment>;
  }
}
