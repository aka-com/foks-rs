/**
 * Isolates screen render failures from the surrounding application shell.
 *
 * A failed screen is replaced with an error notice and retry action while the
 * rail and topbar remain available. The shell keys the boundary by screen and
 * clears failures when the full page identity changes without remounting a
 * healthy screen.
 */

import { Component, Fragment } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { normalizeCommandError } from '../bridge';
import { Band, Button } from '../components';
import type { Location } from '../location';

/**
 * Returns the router fields that identify a screen for boundary remounting.
 * Chat references and Settings or Devices sections identify pages within a
 * screen and are included by `screenIdentity` instead.
 */
export function screenBoundaryKey(location: Location): string {
  return [
    location.kind,
    'ref' in location && location.kind !== 'chat' ? location.ref : '',
  ].join(':');
}

/**
 * Returns the location fields used to clear a displayed screen failure when
 * navigation changes the current page. Healthy screens retain their state.
 */
export function screenIdentity(location: Location): string {
  return [
    screenBoundaryKey(location),
    'ref' in location ? location.ref : '',
    'section' in location ? location.section : '',
    'store' in location ? location.store : '',
    'profile' in location ? location.profile : '',
    'device' in location ? location.device : '',
    'tab' in location ? location.tab : '',
    'channel' in location ? location.channel : '',
  ]
    .map((part) => part ?? '')
    .join(':');
}

interface ScreenErrorBoundaryProps {
  children: ReactNode;
  /** The page's identity; a failure is cleared when it changes. */
  identity?: string;
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

  override componentDidUpdate(previous: ScreenErrorBoundaryProps): void {
    if (this.state.failure && previous.identity !== this.props.identity)
      this.setState((current) => ({
        failure: null,
        generation: current.generation + 1,
      }));
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
