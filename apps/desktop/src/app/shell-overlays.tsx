import {
  useCallback,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from 'react';
import type { ToastController } from '/kit/toasts';
import { normalizeCommandError, type Bridge } from '../bridge';
import type { LocationStore } from '../location';
import { nameOf, storeOf, type AgentSnapshot } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { NavigationPrompt } from '../navigation-guard';
import type { DropUpload } from '../screens/items-screen';
import {
  DEFAULT_READ_ROLE,
  DEFAULT_WRITE_ROLE,
  droppedFileDraft,
  WriteOverlay,
  type WriteWorkflow,
} from '../screens/write-workflows';
import {
  AgentLostCard,
  AgentStopCard,
  Takeover,
  type ShellBlock,
} from './blocking-shell';
import type { CommandErrorHandler } from './catalog-runtime';
import type { useShellNavigation } from './navigation-runtime';

export interface ResumeDraft {
  store: string;
  path: string;
  value: string;
  epoch: number;
}

export function useDroppedUpload({
  bridge,
  shown,
  refresh,
  mutationError,
  setWorkflow,
}: {
  bridge: Bridge;
  shown: AgentSnapshot;
  refresh: (message: string) => Promise<void>;
  mutationError: MutationFailureHandler;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
}) {
  /**
   * Saves a file dropped on a vault's content area. Group items take the same
   * default roles the new-item sheet offers; a path already in use reopens
   * that sheet's conflict step with the dropped file still chosen.
   */
  const uploadDroppedFile = useCallback(
    async ({ storeId, path, sourcePath }: DropUpload): Promise<void> => {
      const target = storeOf(shown, storeId);
      try {
        await bridge.importDroppedFile({
          storeId,
          path,
          sourcePath,
          ...(target?.kind === 'team'
            ? { readRole: DEFAULT_READ_ROLE, writeRole: DEFAULT_WRITE_ROLE }
            : {}),
        });
        await refresh(
          `Uploaded ${nameOf(path)}${target ? ` to ${target.name}` : ''}`,
        );
      } catch (error) {
        if (normalizeCommandError(error).code === 'already-exists') {
          setWorkflow({
            kind: 'exists',
            itemKind: 'Document',
            storeId,
            path,
            draft: droppedFileDraft(path, sourcePath),
          });
          await mutationError(error, { report: false });
        } else await mutationError(error);
      }
    },
    [bridge, mutationError, refresh, shown, setWorkflow],
  );
  return uploadDroppedFile;
}

export function ShellOverlays({
  shown,
  bridge,
  accessNow,
  workflow,
  setWorkflow,
  refresh,
  commandError,
  mutationError,
  refreshSnapshot,
  setResumeDraft,
  setConcealSignal,
  setLatest,
  toasts,
  locations,
  prompt,
  settlePrompt,
  block,
  recoverAgentReadiness,
}: {
  shown: AgentSnapshot;
  bridge: Bridge;
  accessNow: () => number;
  workflow: WriteWorkflow;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
  refresh: (message: string) => Promise<void>;
  commandError: CommandErrorHandler;
  mutationError: MutationFailureHandler;
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  setResumeDraft: Dispatch<SetStateAction<ResumeDraft | null>>;
  setConcealSignal: Dispatch<SetStateAction<number>>;
  setLatest: Dispatch<SetStateAction<AgentSnapshot>>;
  toasts: ToastController;
  locations: LocationStore;
  prompt: ReturnType<typeof useShellNavigation>['prompt'];
  settlePrompt: (confirmed: boolean) => void;
  block: ShellBlock | null;
  recoverAgentReadiness: (reconnect: boolean) => Promise<void>;
}): ReactNode {
  return (
    <>
      <WriteOverlay
        snapshot={shown}
        accessNow={accessNow}
        bridge={bridge}
        workflow={workflow}
        setWorkflow={setWorkflow}
        onApplied={refresh}
        onError={commandError}
        onMutationError={mutationError}
        onRefreshConflict={async (item, draft) => {
          await refreshSnapshot();
          setResumeDraft({
            store: item.store,
            path: item.path,
            value: draft,
            epoch: Date.now(),
          });
          toasts.show('Catalog refreshed. Review your draft.');
        }}
        onDiscardConflict={() => {
          setWorkflow(null);
          setResumeDraft(null);
          setConcealSignal((value) => value + 1);
        }}
        onOpenExisting={async (existing) => {
          const next = await refreshSnapshot();
          const current = next.items.find(
            (item) =>
              item.store === existing.storeId && item.path === existing.path,
          );
          if (!current) {
            throw new Error(
              'That path is now available. Your draft has been saved. Choose a different path or retry creating the item.',
            );
          }
          setLatest(next);
          // The write workflow resolved this destination and is closing, so
          // underlying screens must not block this navigation.
          locations.select(
            { store: current.store, path: current.path },
            { force: true },
          );
        }}
      />
      {prompt ? (
        <NavigationPrompt
          verdict={prompt.verdict}
          onConfirm={() => settlePrompt(true)}
          onCancel={() => settlePrompt(false)}
        />
      ) : null}
      {block ? (
        <Takeover block={block}>
          {block.kind === 'stop' ? (
            <AgentStopCard
              lifecycle={block.lifecycle}
              bridge={bridge}
              onRetryRestoration={() => {
                void recoverAgentReadiness(true).catch(commandError);
              }}
            />
          ) : block.kind === 'disconnected' ? (
            <AgentLostCard
              message={block.message}
              bridge={bridge}
              onRetryAgent={() => recoverAgentReadiness(true)}
            />
          ) : null}
        </Takeover>
      ) : null}
    </>
  );
}
