import { useSyncExternalStore } from 'react';
import type { LocationStore } from './location-store';
import type { LocationState } from './types';

/** Subscribe a component to the navigation state. */
export function useLocationState(store: LocationStore): LocationState {
  return useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
}
