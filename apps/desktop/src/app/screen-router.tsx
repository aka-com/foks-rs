import { Suspense } from 'react';
import type { Dispatch, ReactNode, SetStateAction } from 'react';
import type { AgentLifecycle } from '../agent-lifecycle';
import type { LocationStore } from '../location';
import { storeOf } from '../model';
import type { DeviceLabel, Item } from '../model';
import { markProfileRostersStale } from '../data/roster-staleness';
import { ItemsScreen } from '../screens/items-screen';
import type { DropUpload } from '../screens/files/vault-drop-zone';
import { PlaceholderScreen } from '../screens/placeholder-screen';
import { listsItems } from '../model/scope';
import { useShellServices, useShellSnapshot } from '../shell-services';
import type { WriteWorkflow } from '../screens/files/write-workflow-model';
import {
  ChatTab,
  DevicesScreen,
  GroupSettingsScreen,
  ScreenLoading,
  SettingsScreen,
  TeamsScreen,
  usePreloadVaultScreens,
} from './deferred-screens';
import type { TrashActions } from './trash-actions';

export function ScreenRouter({
  state,
  locations,
  enteredScene,
  namedState,
  concealSignal,
  hardwareRefresh,
  setDeviceLabel,
  setRevealRequest,
  workflow,
  setWorkflow,
  onLock,
  agentLifecycle,
  recoverAgentReadiness,
  uploadDroppedFile,
  accessGenerations,
  onTeamInfo,
  trash,
}: {
  onTeamInfo: (storeId: string, trigger?: HTMLElement) => void;
  trash: TrashActions;
  state: ReturnType<LocationStore['getSnapshot']>;
  locations: LocationStore;
  enteredScene: string;
  namedState: string;
  concealSignal: number;
  hardwareRefresh: number;
  setDeviceLabel: (label: DeviceLabel | null) => void;
  setRevealRequest: (request: string) => void;
  workflow: WriteWorkflow;
  setWorkflow: Dispatch<SetStateAction<WriteWorkflow>>;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  recoverAgentReadiness: (reconnect: boolean) => Promise<void>;
  uploadDroppedFile: (drop: DropUpload) => Promise<void>;
  accessGenerations: ReadonlyMap<string, number>;
}): ReactNode {
  const shown = useShellSnapshot();
  usePreloadVaultScreens();
  const { bridge, accessNow, mutationError, refresh } = useShellServices();
  const here = state.location;
  // An exposure review's Open: the item's store, with the item selected so
  // the details panel shows it and its Edit.
  const openItem = (item: Item): void =>
    locations.navigateAndSelect(
      { kind: 'store', ref: item.store },
      { store: item.store, path: item.path },
    );
  // Every screen but Files loads its chunk on first use; the fallback fills
  // the content pane until it has.
  const screen = listsItems(here) ? (
    <ItemsScreen
      state={state}
      locations={locations}
      onReveal={(item) => setRevealRequest(`${item.store}|${item.path}`)}
      onNew={(itemKind, storeId, initialFolder) =>
        setWorkflow({ kind: 'new', itemKind, storeId, initialFolder })
      }
      onResume={async (storeId) => {
        try {
          // Resuming creation moves the team's chain, so its profile reads
          // the roster again even if the agent's catalog still reports the
          // sequence this write has just moved.
          const profile = storeOf(shown, storeId)?.server;
          if (profile) markProfileRostersStale(profile);
          await bridge.resumeGroupCreation(storeId);
          await refresh('Team creation resumed', profile);
        } catch (error) {
          await mutationError(error);
        }
      }}
      onTeamInfo={onTeamInfo}
      onNewFolder={(storeId, initialFolder) =>
        setWorkflow({ kind: 'new-folder', storeId, initialFolder })
      }
      onMove={(item) => setWorkflow({ kind: 'move', item })}
      onCopyToVault={(item) => setWorkflow({ kind: 'copy', item })}
      onImport={(storeId, initialFolder) =>
        setWorkflow({ kind: 'import', storeId, initialFolder })
      }
      onExport={(storeId) => setWorkflow({ kind: 'export', storeId })}
      onMoveMany={(items, folder) =>
        setWorkflow(
          folder === undefined
            ? { kind: 'move-many', items }
            : { kind: 'bulk-move', items, folder },
        )
      }
      onTrash={trash.moveToTrash}
      onTrashMany={(items) => setWorkflow({ kind: 'bulk-trash', items })}
      onRestore={trash.restore}
      onDeletePermanently={(item) => setWorkflow({ kind: 'delete', item })}
      onEmptyTrash={(store) => setWorkflow({ kind: 'empty-trash', store })}
      onSettings={(storeId) =>
        locations.navigate({
          kind: 'group-settings',
          ref: storeId,
          tab: 'people',
        })
      }
      onUploadDroppedFile={uploadDroppedFile}
      // A modal workflow owns the drop while it is open; the new-item sheet
      // takes dropped files itself.
      dropEnabled={workflow === null}
    />
  ) : here.kind === 'chat' ? (
    // The tab owns the team column and keeps it across a switch; the
    // conversation beside it is what remounts with the team.
    <ChatTab
      key={`chat:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      accessNow={accessNow}
      // The generation belongs to the server of the team the tab opens, and
      // with no `ref` the tab is the only thing that knows which team that is,
      // so it is handed the whole map and picks from the `ref` it resolved.
      accessGenerations={accessGenerations}
      query={state.query}
      onNavigate={(location, options) => locations.navigate(location, options)}
    />
  ) : here.kind === 'group-settings' ? (
    <GroupSettingsScreen
      key={`${here.kind}:${here.ref}`}
      location={here}
      // The Channels tab reads chat, so it guards a new channel on the same
      // access generation the Chat tab does.
      accessGenerations={accessGenerations}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onOpenItem={openItem}
    />
  ) : here.kind === 'teams' ? (
    <TeamsScreen
      key={`teams:${concealSignal}`}
      location={here}
      scene={namedState}
      onNavigate={(location, options) => locations.navigate(location, options)}
    />
  ) : here.kind === 'devices' ? (
    <DevicesScreen
      hardwareRefresh={hardwareRefresh}
      onDeviceLabel={setDeviceLabel}
      key={`devices:${concealSignal}`}
      location={here}
      scene={enteredScene}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onOpenItem={openItem}
    />
  ) : here.kind === 'settings' ? (
    <SettingsScreen
      key={`settings:${concealSignal}`}
      location={here}
      scene={enteredScene}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onLock={onLock}
      agentLifecycle={agentLifecycle}
      onRetryAgent={() => recoverAgentReadiness(true)}
    />
  ) : (
    <PlaceholderScreen location={here} />
  );
  return <Suspense fallback={<ScreenLoading />}>{screen}</Suspense>;
}
