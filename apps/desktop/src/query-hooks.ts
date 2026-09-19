import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from 'react';
import { MetadataRepository, RetiredQueryError } from './metadata-repository';
import type {
  MetadataQuery,
  QueryLoadOptions,
  QuerySnapshot,
} from './metadata-repository';

export const MetadataRepositoryContext =
  createContext<MetadataRepository | null>(null);

/** The unlocked shell provides the shared scope; standalone views own a fallback. */
export function useMetadataRepository(
  identity: object,
  fallback?: MetadataRepository,
): MetadataRepository {
  const shared = useContext(MetadataRepositoryContext);
  const local = useMemo(
    () => fallback ?? new MetadataRepository(),
    // Bridge identity defines the fallback scope lifetime, not a read argument.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [fallback, identity],
  );
  useEffect(
    () => () => {
      if (!fallback) local.clear();
    },
    [fallback, local],
  );
  return shared ?? local;
}

export const QueryRepositoryContext = MetadataRepositoryContext;
export const useQueryRepository = useMetadataRepository;

const EMPTY: QuerySnapshot<never> = Object.freeze({
  data: undefined,
  error: undefined,
  fetching: false,
  invalidation: 0,
});
const empty = () => EMPTY;
const noSubscription = () => () => {};

export function useMetadataQuery<T>(
  query: MetadataQuery<T> | null,
  options: QueryLoadOptions & { onError?: (error: unknown) => void } = {},
): QuerySnapshot<T> {
  const callbacks = useRef(options);
  callbacks.current = options;
  const state = useSyncExternalStore(
    query?.subscribe ?? noSubscription,
    query?.getSnapshot ?? empty,
  );
  useEffect(() => {
    if (!query) return;
    let active = true;
    void query
      .load({ recover: callbacks.current.recover })
      .catch((error: unknown) => {
        if (
          active &&
          !(error instanceof RetiredQueryError) &&
          callbacks.current.onError &&
          query.claimError(error)
        ) {
          callbacks.current.onError(error);
        }
      });
    return () => {
      active = false;
    };
  }, [query, state.invalidation]);
  return state;
}

const NO_STATES: readonly QuerySnapshot<never>[] = Object.freeze([]);

/**
 * Subscribes to a list of queries at once, for a page whose rows are one
 * query each: hooks cannot run in a loop, so the list shares one
 * subscription and one load effect. The caller keeps the list's identity
 * stable between renders; a new list resubscribes. Each entry is loaded on
 * mount and again whenever it is invalidated, and a failure is reported once
 * through `onError`, as `useMetadataQuery` does for one query.
 */
export function useMetadataQueries<T>(
  queries: readonly MetadataQuery<T>[],
  options: QueryLoadOptions & { onError?: (error: unknown) => void } = {},
): readonly QuerySnapshot<T>[] {
  const callbacks = useRef(options);
  callbacks.current = options;
  const subscribe = useCallback(
    (listener: () => void) => {
      const releases = queries.map((query) => query.subscribe(listener));
      return () => {
        for (const release of releases) release();
      };
    },
    [queries],
  );
  // The store's snapshot must keep its identity while nothing changed, so the
  // list is rebuilt only when one of its entries' snapshots has.
  const cached = useRef<{
    queries: readonly MetadataQuery<T>[];
    states: readonly QuerySnapshot<T>[];
  } | null>(null);
  const getSnapshot = useCallback((): readonly QuerySnapshot<T>[] => {
    if (!queries.length) return NO_STATES;
    const states = queries.map((query) => query.getSnapshot());
    const previous = cached.current;
    if (
      previous &&
      previous.queries === queries &&
      previous.states.every((state, index) => state === states[index])
    )
      return previous.states;
    cached.current = { queries, states };
    return states;
  }, [queries]);
  const states = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const invalidations = states.map((state) => state.invalidation).join(',');
  useEffect(() => {
    let active = true;
    for (const query of queries)
      void query
        .load({ recover: callbacks.current.recover })
        .catch((error: unknown) => {
          if (
            active &&
            !(error instanceof RetiredQueryError) &&
            callbacks.current.onError &&
            query.claimError(error)
          )
            callbacks.current.onError(error);
        });
    return () => {
      active = false;
    };
    // A changed invalidation on any entry is a request to load it again; the
    // others answer from the repository's freshness window.
  }, [queries, invalidations]);
  return states;
}
