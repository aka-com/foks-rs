import { createContext, useContext, useRef } from 'react';
import type { ReactNode } from 'react';
import type { AgentSnapshot } from './model/types';
import {
  requireWorkflow, workflowAvailability, workflowMessage,
} from './model/workflow-availability';
import type { WorkflowOperation, WorkflowTarget } from './model/workflow-availability';

const WorkflowContext = createContext<AgentSnapshot | undefined>(undefined);

export function WorkflowProvider({ snapshot, children }: {
  snapshot: AgentSnapshot;
  children: ReactNode;
}) {
  return <WorkflowContext.Provider value={snapshot}>{children}</WorkflowContext.Provider>;
}

export function useWorkflowAccess(snapshot?: AgentSnapshot) {
  const context = useContext(WorkflowContext);
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
    async run<T>(operation: WorkflowOperation, target: WorkflowTarget, task: () => Promise<T>): Promise<T> {
      requireWorkflow(latest.current, operation, target);
      return task();
    },
  };
}
