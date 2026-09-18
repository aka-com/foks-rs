import { normalizeCommandError } from './bridge';
import type {
  MetadataRepository,
  QueryLoadContext,
  QueryLoadOptions,
} from './metadata-repository';
import { RetiredQueryError } from './metadata-repository';

export interface CatalogReadRecovery {
  refresh: () => Promise<unknown>;
  allowed?: () => boolean;
}

/** One catalog repair shared by all reads started before the same repair. */
export class ReadRecoveryCoordinator {
  private revision = 0;
  private pending: Promise<unknown> | null = null;

  options(recovery?: CatalogReadRecovery): QueryLoadOptions {
    const observed = this.revision;
    return {
      recover: async (error: unknown, context: QueryLoadContext) => {
        const typed = normalizeCommandError(error);
        if (
          !recovery ||
          typed.code !== 'catalog-required' ||
          typed.ambiguous ||
          recovery.allowed?.() === false
        )
          return false;
        if (!context.isCurrent()) throw new RetiredQueryError();
        if (this.revision === observed) {
          if (!this.pending) {
            context.repairAttempt();
            const pending = Promise.resolve()
              .then(recovery.refresh)
              .then(() => {
                this.revision++;
              })
              .finally(() => {
                if (this.pending === pending) this.pending = null;
              });
            this.pending = pending;
          }
          await this.pending;
        }
        return context.isCurrent();
      },
    };
  }
}

const coordinators = new WeakMap<MetadataRepository, ReadRecoveryCoordinator>();
export function readRecoveryFor(
  repository: MetadataRepository,
): ReadRecoveryCoordinator {
  let coordinator = coordinators.get(repository);
  if (!coordinator) {
    coordinator = new ReadRecoveryCoordinator();
    coordinators.set(repository, coordinator);
  }
  return coordinator;
}
