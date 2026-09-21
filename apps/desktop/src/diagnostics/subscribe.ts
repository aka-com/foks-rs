/**
 * Attaches the renderer's timing log to the services that already report
 * timings: the reconciliation scheduler, the per-profile request queue, the
 * catalog coordinator and the metadata repository. Each observer maps its
 * event to one log record; none can throw into the service it observes.
 */

import type { CatalogCoordinator } from '../catalog-coordinator';
import type { MetadataRepository } from '../metadata-repository';
import { observeProfileWork } from '../scheduling/profile-work';
import type { ReconciliationScheduler } from '../scheduling/reconciliation';
import { diagnosticLog, redactKey, type DiagnosticLog } from './log';

export interface DiagnosticSources {
  log?: DiagnosticLog;
  scheduler?: ReconciliationScheduler;
  /** The bridge, which owns the renderer's per-profile request queues. */
  workOwner?: object;
  coordinator?: Pick<CatalogCoordinator<unknown>, 'observe'>;
  repository?: MetadataRepository;
}

export function subscribeDiagnostics(sources: DiagnosticSources): () => void {
  const log = sources.log ?? diagnosticLog;
  const stops: (() => void)[] = [];
  if (sources.scheduler)
    stops.push(
      sources.scheduler.observe((event) => {
        log.record({
          name: `job.${event.kind}`,
          scope: event.scope ?? undefined,
          phase: event.outcome,
          ...(event.outcome === 'started' ? {} : { ms: event.milliseconds }),
          outcome:
            event.outcome === 'success'
              ? 'ok'
              : event.outcome === 'failed'
                ? 'error'
                : event.outcome === 'retired'
                  ? 'retired'
                  : undefined,
          attrs: {
            trigger: event.trigger,
            retry: event.retry,
            ...(event.outcome === 'started' ? { late: event.late } : {}),
          },
        });
      }),
    );
  if (sources.workOwner)
    stops.push(
      observeProfileWork(sources.workOwner, (event) => {
        log.record({
          name: 'queue',
          scope: event.profile,
          ms: event.executionMilliseconds,
          outcome:
            event.outcome === 'success'
              ? 'ok'
              : event.busy
                ? 'busy'
                : event.outcome === 'cancelled'
                  ? 'cancelled'
                  : 'error',
          attrs: {
            priority: event.priority,
            wait: event.queueMilliseconds,
            ...(event.key ? { key: redactKey(event.key) } : {}),
          },
        });
      }),
    );
  if (sources.coordinator)
    stops.push(
      sources.coordinator.observe((event) => {
        log.record({
          name: 'catalog.read',
          phase: event.outcome,
          ms: event.milliseconds,
          outcome:
            event.outcome === 'published'
              ? 'ok'
              : event.outcome === 'failed'
                ? 'error'
                : 'retired',
          attrs: { forced: event.forced, profiles: event.profiles },
        });
      }),
    );
  if (sources.repository)
    stops.push(
      sources.repository.observe((event) => {
        // The repository reports no keys: its queries name accounts and
        // invitations, which the popover does not print.
        log.record({
          name: 'metadata.query',
          phase: event.kind,
          ...(event.milliseconds !== undefined
            ? { ms: event.milliseconds }
            : {}),
          outcome:
            event.kind === 'load-complete'
              ? 'ok'
              : event.kind === 'load-error'
                ? 'error'
                : undefined,
        });
      }),
    );
  return () => {
    for (const stop of stops) stop();
  };
}
