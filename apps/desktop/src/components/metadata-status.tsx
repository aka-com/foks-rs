import type { ReactNode } from 'react';
import type { metadataFreshness } from '../device-cache';

export function MetadataStatus({
  label,
  freshness,
}: {
  label: string;
  freshness: ReturnType<typeof metadataFreshness>;
}): ReactNode {
  if (
    !freshness.stale &&
    !freshness.refreshing &&
    freshness.lastSuccessAt === undefined
  )
    return null;
  return (
    <p className="fn" aria-label={`${label} freshness`}>
      {label}:{' '}
      {freshness.stale
        ? freshness.complete
          ? 'Refresh failed; showing previously loaded data.'
          : 'Some data could not be loaded.'
        : freshness.refreshing
          ? 'Refreshing.'
          : 'Loaded.'}
      {freshness.lastSuccessAt !== undefined
        ? ` Last successful read: ${new Date(freshness.lastSuccessAt).toLocaleTimeString()}.`
        : ''}
    </p>
  );
}
