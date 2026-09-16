import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from 'react';
import { QueryRepository, RetiredQueryError } from './query-repository';
import type {
  MetadataQuery,
  QueryLoadOptions,
  QuerySnapshot,
} from './query-repository';

export const QueryRepositoryContext = createContext<QueryRepository | null>(
  null,
);

/** The unlocked shell provides the shared scope; standalone views own a fallback. */
export function useQueryRepository(
  identity: object,
  fallback?: QueryRepository,
): QueryRepository {
  const shared = useContext(QueryRepositoryContext);
  const local = useMemo(
    () => fallback ?? new QueryRepository(),
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
