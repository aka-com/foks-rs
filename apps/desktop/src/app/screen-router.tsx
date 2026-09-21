import type { Dispatch, ReactNode, SetStateAction } from 'react';
import type { AgentLifecycle } from '../agent-lifecycle';
import type { Bridge } from '../bridge';
import type { Location, LocationStore, NavigateOptions } from '../location';
import { storeOf } from '../model';
import type { AgentSnapshot, DeviceLabel } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { markProfileRostersStale } from '../roster-staleness';
import { ChatTab } from '../screens/chat-tab';
import { DevicesScreen } from '../screens/devices-screen';
import { GroupSettingsScreen } from '../screens/groups-screen';
import { ItemsScreen, type DropUpload } from '../screens/items-screen';
import { PlaceholderScreen } from '../screens/placeholder-screen';
import { listsItems } from '../screens/scope';
import { SettingsScreen } from '../screens/settings-screen';
import { TeamsScreen } from '../screens/teams-screen';
import type { WriteWorkflow } from '../screens/write-workflows';
import type { CommandErrorHandler } from './catalog-runtime';

export function ScreenRouter({
  shown,
  bridge,
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
  refresh,
  refreshSnapshot,
  commandError,
  mutationError,
  onLock,
  agentLifecycle,
  recoverAgentReadiness,
  uploadDroppedFile,
  accessNow,
  accessGenerations,
}: {
  shown: AgentSnapshot;
  bridge: Bridge;
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
  refresh: (message: string, profile?: string) => Promise<void>;
  refreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  commandError: CommandErrorHandler;
  mutationError: MutationFailureHandler;
  onLock: () => Promise<boolean>;
  agentLifecycle: AgentLifecycle;
  recoverAgentReadiness: (reconnect: boolean) => Promise<void>;
  uploadDroppedFile: (drop: DropUpload) => Promise<void>;
  accessNow: () => number;
  accessGenerations: ReadonlyMap<string, number>;
}): ReactNode {
  const here = state.location;
  // People, Teams, Devices and Settings are the same body under four titles.
  const settingsProps = {
    snapshot: shown,
    bridge,
    scene: enteredScene,
    onNavigate: (location: Location, options?: NavigateOptions) =>
      locations.navigate(location, options),
    onRefresh: refresh,
    onRefreshSnapshot: refreshSnapshot,
    onError: commandError,
    onMutationError: mutationError,
    onLock,
    agentLifecycle,
    onRetryAgent: () => recoverAgentReadiness(true),
  };
  return listsItems(here) ? (
    <ItemsScreen
      snapshot={shown}
      bridge={bridge}
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
      onDelete={(item) => setWorkflow({ kind: 'delete', item })}
      onSettings={(storeId) =>
        locations.navigate({
          kind: 'group-settings',
          ref: storeId,
          tab: 'people',
        })
      }
      onCommandError={commandError}
      onUploadDroppedFile={uploadDroppedFile}
      // A modal workflow owns the drop while it is open; the new-item sheet
      // takes dropped files itself.
      dropEnabled={workflow === null}
      accessNow={accessNow}
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
      snapshot={shown}
      bridge={bridge}
      location={here}
      // The Channels tab reads chat, so it decides availability on the shell's
      // own clock and guards a new channel on the same access generation the
      // Chat tab does.
      accessNow={accessNow}
      accessGenerations={accessGenerations}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onApplied={refresh}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'teams' ? (
    // Teams shares no state with Settings, so it is given exactly the props it
    // declares rather than the settings bundle.
    <TeamsScreen
      key={`teams:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      scene={namedState}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onRefresh={refresh}
      onRefreshSnapshot={refreshSnapshot}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'devices' ? (
    <DevicesScreen
      hardwareRefresh={hardwareRefresh}
      onDeviceLabel={setDeviceLabel}
      key={`devices:${concealSignal}`}
      snapshot={shown}
      bridge={bridge}
      location={here}
      scene={enteredScene}
      onNavigate={(location, options) => locations.navigate(location, options)}
      onRefresh={refresh}
      onRefreshSnapshot={refreshSnapshot}
      onError={commandError}
      onMutationError={mutationError}
    />
  ) : here.kind === 'settings' ? (
    <SettingsScreen
      key={`settings:${concealSignal}`}
      {...settingsProps}
      location={here}
    />
  ) : (
    <PlaceholderScreen location={here} />
  );
}
