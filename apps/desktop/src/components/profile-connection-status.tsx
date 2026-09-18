import type { ReactNode } from 'react';
import { Button } from './index';
import { connectionObservationFresh } from '../profile-connectivity';
import type { ProfileConnectivity } from '../profile-connectivity';

export function ProfileConnectionStatus({
  observation,
  name,
  busy,
  disabled,
  onRetry,
  nowSeconds = Date.now() / 1_000,
}: {
  observation: ProfileConnectivity;
  name: string;
  busy?: boolean;
  disabled?: boolean;
  onRetry?: () => void;
  nowSeconds?: number;
}): ReactNode {
  if (observation.status === 'unknown' && !onRetry) return null;
  return (
    <div className="profile-connection-status">
      {observation.status === 'unknown' ? (
        <span>Live reachability has not been observed.</span>
      ) : (
        <>
          <span>
            Identity check:{' '}
            {observation.identity.status === 'connected'
              ? 'verified response'
              : observation.identity.error.message}
            .
          </span>{' '}
          <span>
            Compatibility:{' '}
            {observation.compatibility.status === 'failed'
              ? observation.compatibility.error.message
              : observation.compatibility.status === 'not-required'
                ? 'no renewal required'
                : observation.compatibility.status === 'renewed'
                  ? 'artifact updated'
                  : 'artifact unchanged'}
            .
          </span>{' '}
          <span>
            Observed{' '}
            {new Date(observation.observedAt * 1_000).toLocaleTimeString()}
            {connectionObservationFresh(observation, nowSeconds, 120)
              ? ''
              : ' (stale)'}
            .
          </span>
        </>
      )}
      {onRetry ? (
        <Button
          size="sm"
          disabled={disabled || busy}
          onClick={onRetry}
          aria-label={`Reconnect ${name}`}
        >
          {busy ? 'Reconnecting' : 'Reconnect'}
        </Button>
      ) : null}
    </div>
  );
}
