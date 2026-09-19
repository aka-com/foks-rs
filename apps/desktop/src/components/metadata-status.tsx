/**
 * A one-line caption under a list whose data may be stale.
 *
 * Fresh data draws nothing. A refresh in progress is a spinner alone. Only a
 * failure produces a sentence: when the data was last read, that it could not
 * be refreshed, and a Retry when the page can ask again. The caption sits under
 * the list it describes rather than at the top of the page, so it says which
 * list is affected when a page has several.
 */

import type { ReactNode } from 'react';
import type { metadataFreshness } from '../device-cache';

function timeOf(milliseconds: number): string {
  return new Date(milliseconds).toLocaleTimeString([], {
    hour: 'numeric',
    minute: '2-digit',
  });
}

export function FreshnessCaption({
  label,
  freshness,
  onRetry,
}: {
  /** What the list holds, lowercase: "device metadata", "security keys". */
  label: string;
  freshness: ReturnType<typeof metadataFreshness>;
  onRetry?: () => void;
}): ReactNode {
  if (freshness.stale) {
    const sentence =
      freshness.lastSuccessAt === undefined
        ? `${label} could not be loaded`
        : `As of ${timeOf(freshness.lastSuccessAt)} · ${label} could not be refreshed`;
    return (
      <p className="freshness stale" aria-label={`${label} freshness`}>
        <span className="dot" aria-hidden="true" />
        <span>{sentence[0].toUpperCase() + sentence.slice(1)}</span>
        {onRetry ? (
          <>
            <span aria-hidden="true">·</span>
            <button type="button" className="lnk" onClick={onRetry}>
              Retry
            </button>
          </>
        ) : null}
      </p>
    );
  }
  if (freshness.refreshing && !freshness.complete)
    return (
      <p
        className="freshness refreshing"
        role="status"
        aria-label={`${label} refreshing`}
      >
        <span className="spin" aria-hidden="true" />
      </p>
    );
  return null;
}
