import { useContext, useRef } from 'react';
import type { AgentSnapshot } from './model/types';
import {
  requireWorkflow,
  workflowAvailability,
  workflowMessage,
} from './model/workflow-availability';
import type {
  WorkflowOperation,
  WorkflowTarget,
} from './model/workflow-availability';
import { ShellSnapshotContext } from './shell-services';

/**
 * Workflow availability against `snapshot`, or against the shell's snapshot
 * when none is given. Outside a `ShellSnapshotProvider` and without a
 * snapshot, every workflow is unavailable.
 */
export function useWorkflowAccess(snapshot?: AgentSnapshot) {
  const context = useContext(ShellSnapshotContext);
  const latest = useRef(snapshot ?? context);
  latest.current = snapshot ?? context;
  const availability = (operation: WorkflowOperation, target: WorkflowTarget) =>
    workflowAvailability(latest.current, operation, target);
  return {
    availability,
    props(operation: WorkflowOperation, target: WorkflowTarget) {
      const access = availability(operation, target);
      return { disabled: !access.available, title: workflowMessage(access) };
    },
    require(operation: WorkflowOperation, target: WorkflowTarget) {
      requireWorkflow(latest.current, operation, target);
    },
    async run<T>(
      operation: WorkflowOperation,
      target: WorkflowTarget,
      task: () => Promise<T>,
    ): Promise<T> {
      requireWorkflow(latest.current, operation, target);
      return task();
    },
  };
}
