import {
  useCallback,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from 'react';
import type { ToastController } from '../../kit/toasts';
import { normalizeCommandError, type Bridge } from '../bridge';
import type { LocationStore } from '../location';
import { nameOf, storeOf, type AgentSnapshot, type Item } from '../model';
import type { MutationFailureHandler } from '../data/mutation-recovery';
import { NavigationPrompt } from '../navigation-guard';
import { useShellServices } from '../shell-services';
import type { DropUpload } from '../screens/files/vault-drop-zone';
import { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE } from '../screens/item-roles';
import {
  conflictDraftWorkflow,
  droppedFileDraft,
  type WriteWorkflow,
} from '../screens/write-workflow-model';
import { WriteOverlay } from '../screens/write-workflows';
import {
  AgentLostCard,
  AgentStopCard,
  Takeover,
  type ShellBlock,
} from './blocking-shell';
import { PanelErrorBoundary } from './fault-boundaries';
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
  refresh: (message: string, profile?: string) => Promise<void>;
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
          target?.server,
        );
      } catch (error) {
        if (normalizeCommandError(error).code === 'already-exists') {
          setWorkflow({
            kind: 'exists',
            itemKind: 'Document',
            storeId,
            path,
            initialFolder: path.slice(0, path.lastIndexOf('/')) || '/',
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
  workflow,
  setWorkflow,
  restoreTrash,
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
  workflow: WriteWorkflow;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
  restoreTrash: (item: Item, destination: string) => Promise<void>;
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
  const { bridge, commandError, refreshSnapshot } = useShellServices();
  return (
    <>
      {/* A workflow sheet that fails to render is closed; the navigation
          prompt and the blocking takeover below stay outside its boundary. */}
      <PanelErrorBoundary
        label="write-workflow"
        identity={workflow?.kind ?? ''}
        onFault={() => {
          setWorkflow(null);
          toasts.show('The dialog could not be shown and was closed.', {
            tone: 'warning',
          });
        }}
      >
        <WriteOverlay
          workflow={workflow}
          setWorkflow={setWorkflow}
          onRefreshConflict={async (item, draft, operation) => {
            const next = await refreshSnapshot();
            const current = next.items.find(
              (candidate) =>
                candidate.store === item.store && candidate.path === item.path,
            );
            // The agent reports one conflict whether the item changed or was
            // removed; the refresh tells them apart. A removed item has no
            // newer version to review, and the details panel would have
            // nothing to resume the draft into, so the edit becomes a
            // new-item draft at the same path instead of being lost.
            if (!current) {
              if (operation === 'replace') {
                toasts.show('This file was deleted elsewhere.');
                return null;
              }
              toasts.show(
                'This item was deleted elsewhere. Your edit is kept as a new item at the same path.',
              );
              return conflictDraftWorkflow(item, draft);
            }
            if (operation === 'replace') {
              toasts.show(
                'Catalog refreshed. Review the current file, then choose its replacement again.',
              );
              return null;
            }
            setResumeDraft({
              store: item.store,
              path: item.path,
              value: draft,
              epoch: Date.now(),
            });
            toasts.show('Catalog refreshed. Review your draft.');
            return null;
          }}
          onDeleteConflict={async (item) => {
            const next = await refreshSnapshot(true);
            const current = next.items.find(
              (candidate) =>
                candidate.store === item.store && candidate.path === item.path,
            );
            if (current)
              toasts.show(
                'This item was modified by another user or session. Review the updated item before deleting.',
                { tone: 'warning' },
              );
            else toasts.show('This item was already deleted elsewhere.');
          }}
          onRestoreTrash={restoreTrash}
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
      </PanelErrorBoundary>
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
              credentialsRequired={block.credentialsRequired}
              bridge={bridge}
              onRetryAgent={() => recoverAgentReadiness(true)}
            />
          ) : null}
        </Takeover>
      ) : null}
    </>
  );
}
