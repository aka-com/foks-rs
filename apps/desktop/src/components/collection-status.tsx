import type { CollectionReadiness } from '../model';

/** Initial and incomplete collection reads must not look like empty results. */
export function CollectionStatus({
  state,
  label,
}: {
  state: CollectionReadiness;
  label: string;
}) {
  if (state === 'ready') return null;
  return (
    <p className="fn" role={state === 'loading' ? 'status' : 'alert'}>
      {state === 'loading' ? (
        <>Loading {label}…</>
      ) : (
        <>Could not load {label}. Refresh to try again.</>
      )}
    </p>
  );
}
