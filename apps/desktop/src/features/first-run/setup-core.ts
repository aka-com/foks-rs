import type { Bridge } from '../../bridge';
import type { FirstRunFailure, FirstRunOperation } from './first-run-failure';
import type { AgentSnapshot } from '../../model';
import type { useFirstRunController } from './use-first-run-controller';

/**
 * Reports a failed setup operation. `report` shows the message (the session
 * message by default), `isCurrent` drops a report from a workflow that was
 * superseded, and `onFailure` receives the classified failure and, after a
 * reconciliation, its reconciled form.
 */
export type SetupFailureReporter = (
  operation: FirstRunOperation,
  error: unknown,
  report?: (message: string) => void,
  isCurrent?: () => boolean,
  onFailure?: (failure: FirstRunFailure) => void,
) => void;

/**
 * What every setup workflow hook reads or calls: the checkpoint controller,
 * the bridge and agent readiness, the shell's catalog reload, and the
 * session's message, failure report and account busy flag.
 * useFirstRunSession builds it once per render and passes it to each
 * workflow, so a workflow's own parameters are only what it adds.
 */
export type SetupCore = Pick<
  ReturnType<typeof useFirstRunController>,
  'checkpoint' | 'checkpointRef' | 'mounted' | 'commit' | 'send'
> & {
  readonly bridge: Bridge;
  readonly agentReady: boolean;
  readonly onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  readonly setMessage: (message: string | null) => void;
  readonly fail: SetupFailureReporter;
  /**
   * Marks an account provisioning, protection or backup write as running;
   * the session reports it as the `account` busy owner.
   */
  readonly setAccountBusy: (busy: boolean) => void;
};
