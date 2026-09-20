import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { chatActionMutates, decodeChatReply } from '../chat-contract';
import { decodeLocalSession } from '../chat/local-contract';
import type { Bridge } from './contract';
import {
  decodeAgentProcessInfo,
  decodeAgentStatus,
  decodeAppInfo,
  decodeAppLockState,
  decodeConnectionLoss,
  decodeDropHover,
  decodeDropPaths,
  decodeMaintenanceSnapshot,
  decodeTimingBatch,
  decodeWindowState,
} from './core';
import { checked, checkedMutation } from './transport';
import { decodeCopy } from './vault-catalog';

export const coreCommands: Pick<
  Bridge,
  | 'cancelChat'
  | 'chat'
  | 'appLockState'
  | 'windowState'
  | 'setTrafficLightsVisible'
  | 'setUnsentMessages'
  | 'lockApp'
  | 'unlockApp'
  | 'restartApp'
  | 'quitApp'
  | 'agentStatus'
  | 'probeAgentStatus'
  | 'appInfo'
  | 'takeAgentConnectionLoss'
  | 'retryAgentConnection'
  | 'autoRecoverAgent'
  | 'chatLocal'
  | 'openNotificationSettings'
  | 'relocateClientState'
  | 'maintainClientState'
  | 'clientStateMaintenanceStatus'
  | 'agentProcessInfo'
  | 'diagnosticTimings'
  | 'restartAgent'
  | 'openChatLink'
  | 'copyText'
  | 'initializeClientState'
  | 'onDropHover'
  | 'onDropPaths'
  | 'onWindowState'
  | 'onChatNotification'
  | 'onOpenSettings'
  | 'onMaintenanceStatus'
> = {
  cancelChat: (viewId) => invoke<void>('cancel_chat_requests', { viewId }),
  chat: (storeId, action, viewId) =>
    (chatActionMutates(action) ? checkedMutation : checked)(
      'chat_request',
      { storeId, action, viewId },
      (value) => decodeChatReply(value, storeId, action),
    ),
  appLockState: () => checked('app_lock_state', undefined, decodeAppLockState),
  windowState: () => checked('get_window_state', undefined, decodeWindowState),
  setTrafficLightsVisible: (visible) =>
    checked('set_traffic_lights_visible', { visible }, () => undefined),
  setUnsentMessages: (count) =>
    checked('set_unsent_messages', { count }, () => undefined),
  lockApp: () => checked('lock_app', undefined, decodeAppLockState),
  unlockApp: () => checked('unlock_app', undefined, decodeAppLockState),
  restartApp: () => invoke<void>('restart_app'),
  quitApp: () => invoke<void>('quit_app'),
  agentStatus: () => checked('agent_status', undefined, decodeAgentStatus),
  probeAgentStatus: () =>
    checked('agent_status', undefined, decodeAgentStatus, false),
  appInfo: () => checked('app_info', undefined, decodeAppInfo),
  takeAgentConnectionLoss: () =>
    checked('take_agent_connection_loss', undefined, decodeConnectionLoss),
  retryAgentConnection: () =>
    checked('retry_agent_connection', undefined, decodeAgentStatus),
  autoRecoverAgent: () =>
    checked('auto_recover_agent', undefined, decodeAgentStatus, false),
  chatLocal: (action) => checked('chat_local', { action }, decodeLocalSession),
  openNotificationSettings: () =>
    checked('open_notification_settings', undefined, decodeCopy),
  relocateClientState: () =>
    checked('relocate_client_state', {}, decodeMaintenanceSnapshot),
  maintainClientState: (action) =>
    checked('maintain_client_state', { action }, decodeMaintenanceSnapshot),
  clientStateMaintenanceStatus: () =>
    checked(
      'client_state_maintenance_status',
      undefined,
      decodeMaintenanceSnapshot,
    ),
  agentProcessInfo: () =>
    checked('agent_process_info', undefined, decodeAgentProcessInfo),
  diagnosticTimings: (since) =>
    checked('diagnostic_timings', { since }, decodeTimingBatch, false),
  restartAgent: (takeover) =>
    checked('restart_agent', { takeover }, decodeMaintenanceSnapshot),
  openChatLink: (url) => checked('open_chat_link', { url }, decodeCopy),
  copyText: (text) => checked('copy_text', { text }, decodeCopy),
  initializeClientState: () =>
    checkedMutation('initialize_client_state', undefined, decodeAgentStatus),
  onDropHover: async (listener) =>
    listen<unknown>('foks://drop-hover', (event) => {
      listener(decodeDropHover(event.payload));
    }),
  onDropPaths: async (listener) =>
    listen<unknown>('foks://drop-paths', (event) => {
      listener(decodeDropPaths(event.payload));
    }),
  onWindowState: async (listener) =>
    listen<unknown>('foks://window-state', (event) => {
      listener(decodeWindowState(event.payload));
    }),
  onChatNotification: async (listener) => {
    const activation = await listen('foks://chat-notification', () =>
      listener('activate'),
    );
    try {
      const error = await listen('foks://chat-notification-error', () =>
        listener('error'),
      );
      return () => {
        activation();
        error();
      };
    } catch (cause) {
      activation();
      throw cause;
    }
  },
  onOpenSettings: async (listener) => listen('foks://open-settings', listener),
  onMaintenanceStatus: async (listener) =>
    listen<unknown>('foks://maintenance-status', (event) => {
      listener(decodeMaintenanceSnapshot(event.payload));
    }),
};
