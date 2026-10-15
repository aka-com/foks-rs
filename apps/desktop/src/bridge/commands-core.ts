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
  decodeExitState,
  decodeMaintenanceSnapshot,
  decodeTimingBatch,
  decodeWindowState,
} from './core';
import type { Transport } from './transport';
import { decodeCopy } from './vault-catalog';

export const coreCommands = (
  transport: Transport,
): Pick<
  Bridge,
  | 'cancelChat'
  | 'chat'
  | 'appLockState'
  | 'windowState'
  | 'setTrafficLightsVisible'
  | 'setUnsentMessages'
  | 'setQuitConfirmation'
  | 'lockApp'
  | 'unlockApp'
  | 'restartApp'
  | 'quitApp'
  | 'exitState'
  | 'handleExitAction'
  | 'agentStatus'
  | 'probeAgentStatus'
  | 'appInfo'
  | 'takeAgentConnectionLoss'
  | 'retryAgentConnection'
  | 'autoRecoverAgent'
  | 'chatLocal'
  | 'openNotificationSettings'
  | 'setUnreadBadge'
  | 'relocateClientState'
  | 'maintainClientState'
  | 'clientStateMaintenanceStatus'
  | 'agentProcessInfo'
  | 'diagnosticTimings'
  | 'restartAgent'
  | 'openChatLink'
  | 'copyText'
  | 'initializeClientState'
  | 'onAgentConnectionLoss'
  | 'onDropHover'
  | 'onDropPaths'
  | 'onWindowState'
  | 'onChatNotification'
  | 'onOpenSettings'
  | 'onNewItem'
  | 'onLockMenu'
  | 'onMaintenanceStatus'
  | 'onExitState'
> => ({
  cancelChat: (viewId) => transport.invoke('cancel_chat_requests', { viewId }),
  chat: (storeId, action, viewId) =>
    (chatActionMutates(action) ? transport.checkedMutation : transport.checked)(
      'chat_request',
      { storeId, action, viewId },
      (value) => decodeChatReply(value, storeId, action),
    ),
  appLockState: () =>
    transport.checked('app_lock_state', undefined, decodeAppLockState),
  windowState: () =>
    transport.checked('get_window_state', undefined, decodeWindowState),
  setTrafficLightsVisible: (visible) =>
    transport.checked(
      'set_traffic_lights_visible',
      { visible },
      () => undefined,
    ),
  setUnsentMessages: (count) =>
    transport.checked('set_unsent_messages', { count }, () => undefined),
  setQuitConfirmation: (confirmation) =>
    transport.checked(
      'set_quit_confirmation',
      { confirmation },
      () => undefined,
    ),
  lockApp: () => transport.checked('lock_app', undefined, decodeAppLockState),
  unlockApp: () =>
    transport.checked('unlock_app', undefined, decodeAppLockState),
  restartApp: () => transport.invoke('restart_app'),
  quitApp: () => transport.invoke('quit_app'),
  exitState: () => transport.checked('exit_state', undefined, decodeExitState),
  handleExitAction: (action) =>
    transport.checked('handle_exit_action', { action }, () => undefined),
  agentStatus: () =>
    transport.checked('agent_status', undefined, decodeAgentStatus),
  probeAgentStatus: () =>
    transport.checked('agent_status', undefined, decodeAgentStatus, false),
  appInfo: () => transport.checked('app_info', undefined, decodeAppInfo),
  takeAgentConnectionLoss: () =>
    transport.checked(
      'take_agent_connection_loss',
      undefined,
      decodeConnectionLoss,
    ),
  retryAgentConnection: () =>
    transport.checked('retry_agent_connection', undefined, decodeAgentStatus),
  autoRecoverAgent: () =>
    transport.checked(
      'auto_recover_agent',
      undefined,
      decodeAgentStatus,
      false,
    ),
  chatLocal: (action) =>
    transport.checked('chat_local', { action }, decodeLocalSession),
  openNotificationSettings: () =>
    transport.checked('open_notification_settings', undefined, decodeCopy),
  setUnreadBadge: (count) =>
    transport.checked('set_unread_badge', { count }, decodeCopy),
  relocateClientState: () =>
    transport.checked('relocate_client_state', {}, decodeMaintenanceSnapshot),
  maintainClientState: (action) =>
    transport.checked(
      'maintain_client_state',
      { action },
      decodeMaintenanceSnapshot,
    ),
  clientStateMaintenanceStatus: () =>
    transport.checked(
      'client_state_maintenance_status',
      undefined,
      decodeMaintenanceSnapshot,
    ),
  agentProcessInfo: () =>
    transport.checked('agent_process_info', undefined, decodeAgentProcessInfo),
  diagnosticTimings: (since) =>
    transport.checked(
      'diagnostic_timings',
      { since },
      decodeTimingBatch,
      false,
    ),
  restartAgent: (takeover) =>
    transport.checked('restart_agent', { takeover }, decodeMaintenanceSnapshot),
  openChatLink: (url) =>
    transport.checked('open_chat_link', { url }, decodeCopy),
  copyText: (text) => transport.checked('copy_text', { text }, decodeCopy),
  initializeClientState: () =>
    transport.checkedMutation(
      'initialize_client_state',
      undefined,
      decodeAgentStatus,
    ),
  onAgentConnectionLoss: async (listener) =>
    transport.listen('foks://agent-connection-loss', () => listener()),
  onDropHover: async (listener) =>
    transport.listen('foks://drop-hover', (payload) => {
      listener(decodeDropHover(payload));
    }),
  onDropPaths: async (listener) =>
    transport.listen('foks://drop-paths', (payload) => {
      listener(decodeDropPaths(payload));
    }),
  onWindowState: async (listener) =>
    transport.listen('foks://window-state', (payload) => {
      listener(decodeWindowState(payload));
    }),
  onChatNotification: async (listener) => {
    const activation = await transport.listen('foks://chat-notification', () =>
      listener('activate'),
    );
    try {
      const error = await transport.listen(
        'foks://chat-notification-error',
        () => listener('error'),
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
  onOpenSettings: async (listener) =>
    transport.listen('foks://open-settings', listener),
  onNewItem: async (listener) => {
    const password = await transport.listen('foks://new-password', () =>
      listener('Password'),
    );
    try {
      const document = await transport.listen('foks://new-document', () =>
        listener('Document'),
      );
      return () => {
        password();
        document();
      };
    } catch (cause) {
      password();
      throw cause;
    }
  },
  onLockMenu: async (listener) => transport.listen('foks://lock', listener),
  onMaintenanceStatus: async (listener) =>
    transport.listen('foks://maintenance-status', (payload) => {
      listener(decodeMaintenanceSnapshot(payload));
    }),
  onExitState: async (listener) =>
    transport.listen('foks://exit-state', (payload) => {
      listener(decodeExitState(payload));
    }),
});
