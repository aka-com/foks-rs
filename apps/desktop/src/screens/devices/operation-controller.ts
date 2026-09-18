import { enqueueProfileWork } from '../../bridge';
import type { Bridge } from '../../bridge';
import type { WorkflowOperation, WorkflowTarget } from '../../model/workflow-availability';
import type { useWorkflowAccess } from '../../workflow-context';
import { attemptMutation, reportMutationOutcome } from '../../commands/command-policy';
import type { MutationPolicy } from '../../commands/command-policy';
import type { CommandError } from '../../bridge';

export function queuedDeviceWork<T>(
  bridge: Bridge,
  profile: string,
  access: ReturnType<typeof useWorkflowAccess>,
  operation: WorkflowOperation,
  target: WorkflowTarget,
  task: () => Promise<T>,
): Promise<T> {
  return enqueueProfileWork(bridge, profile, () => access.run(operation, target, task));
}

export async function runDeviceMutation<T>(
  write: () => Promise<T>,
  onApplied: (value: T) => Promise<void>,
  onFailure: (error: CommandError) => void | Promise<void>,
  onRefreshPending: (error: CommandError) => void | Promise<void>,
  isCurrent: () => boolean,
  policy: MutationPolicy = { kind: 'mutation' },
) {
  let observed = false;
  const result = await attemptMutation(policy, write, async (value) => {
    observed = isCurrent();
    if (observed) await onApplied(value);
  });
  if (result.outcome === 'applied' && !observed)
    return { outcome: 'applied', synchronization: 'unobserved' } as const;
  if (result.outcome === 'applied' || isCurrent())
    await reportMutationOutcome(result, onFailure, onRefreshPending);
  return result;
}
