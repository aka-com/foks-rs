import {
  createContext,
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
