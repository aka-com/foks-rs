/**
 * The mock's application and agent commands, the members of
 * `bridge/commands-core.ts`: the app lock, the window, the exit flow, agent
 * status, the chat, desktop alerts and the host's event listeners. They
 * answer with decoded values.
 */

import type { coreCommands } from '../bridge/commands-core';
import { notificationKey, teamKey } from '../chat/local-contract';
import type { LocalAlertMode } from '../chat/local-contract';
import { mockChat } from './chat-mock';
import { chatMockScenario } from './chat-mock-scenarios';
import type { MockDomain, MockState } from './state';

export type AppLifecycleRequest = 'restart' | 'quit';

export const mockAppLifecycleRequests: AppLifecycleRequest[] = [];

export function mockCoreCommands({
  snapshot,
}: MockState): MockDomain<typeof coreCommands> {
  let connectionLoss =
    typeof window !== 'undefined' &&
    new URLSearchParams(window.location?.search ?? '').get('state') ===
      'agent-lost'
      ? 'The agent socket closed while reading the catalog.'
      : null;
  const hoverListeners = new Set<(event: { hovering: boolean }) => void>();
  const pathListeners = new Set<(paths: string[]) => void>();
  let appLocked = false;
  const notificationSettings = {
    enabled: false,
    previews: false,
    badge: true,
    overrides: {} as Record<string, LocalAlertMode>,
    muted: new Set<string>(),
    teams: {} as Record<string, LocalAlertMode>,
  };
  const appLockState = () => ({
    locked: appLocked,
    available: true,
    mechanism: 'password' as const,
  });
  // `?chat-scenario=busy` (or `failures`, `integrity`) seeds the chat with
  // that scenario instead of one channel holding one message.
  const chat = mockChat(
    snapshot,
    typeof window === 'undefined'
      ? 'default'
      : chatMockScenario(window.location?.search ?? ''),
  );
  return {
    // Optional methods only the native host offers; callers fall back to
    // other paths when a bridge lacks them.
    diagnosticTimings: undefined,
    probeAgentStatus: undefined,
    autoRecoverAgent: undefined,
    maintainClientState: async () => {
      throw new Error('State maintenance requires the native application.');
    },
    clientStateMaintenanceStatus: async () => ({
      state: 'idle',
      generation: 0,
      revision: 0,
    }),
    agentProcessInfo: async () => ({
      pid: 50350,
      executable: '/Applications/FOKS.app/Contents/MacOS/foks-agent',
      startedAt: Math.floor(Date.now() / 1000) - 5 * 3600,
      owned: true,
    }),
    restartAgent: async () => {
      throw new Error('Restarting the agent requires the native application.');
    },
    relocateClientState: async () => {
      throw new Error('State maintenance requires the native application.');
    },
    chat,
    cancelChat: chat.cancel,
    appLockState: async () => appLockState(),
    windowState: async () => ({ maximized: false, fullscreen: false }),
    setTrafficLightsVisible: async () => {},
    setUnsentMessages: async () => {},
    setQuitConfirmation: async () => {},
    lockApp: async () => {
      appLocked = true;
      return appLockState();
    },
    unlockApp: async () => {
      appLocked = false;
      return appLockState();
    },
    restartApp: async () => {
      mockAppLifecycleRequests.push('restart');
    },
    quitApp: async () => {
      mockAppLifecycleRequests.push('quit');
    },
    exitState: async () => ({ state: 'idle' }),
    handleExitAction: async () => {},
    agentStatus: () => Promise.resolve({ ...snapshot.agent }),
    appInfo: async () => ({
      version: '0.4.0',
      agentSocket: '/private/foks/agent.sock',
    }),
    takeAgentConnectionLoss: async () => {
      const pending = connectionLoss;
      connectionLoss = null;
      return pending;
    },
    retryAgentConnection: async () => ({ ...snapshot.agent }),
    chatLocal: async (action) => {
      if (action.action === 'configure') {
        if (action.enabled !== undefined)
          notificationSettings.enabled = action.enabled;
        if (action.previews !== undefined)
          notificationSettings.previews = action.previews;
        if (action.badge !== undefined)
          notificationSettings.badge = action.badge;
        if (action.scope) {
          const [key, modes] = action.channel
            ? [
                notificationKey(action.scope, action.channel),
                notificationSettings.overrides,
              ]
            : [teamKey(action.scope), notificationSettings.teams];
          if (action.mode === null) delete modes[key];
          else if (action.mode !== undefined) modes[key] = action.mode;
          if (action.muted !== undefined) {
            if (action.muted) notificationSettings.muted.add(key);
            else notificationSettings.muted.delete(key);
          }
        }
      }
      return {
        ...(action.action === 'recover-intents'
          ? { migrationRemaining: 0 }
          : {}),
        epoch: '0'.repeat(32),
        available: true,
        backend: 'none',
        settings: {
          ...notificationSettings,
          overrides: { ...notificationSettings.overrides },
          muted: [...notificationSettings.muted],
          teams: { ...notificationSettings.teams },
        },
      };
    },
    openChatLink: async () => ({ ok: true }),
    openNotificationSettings: async () => ({ ok: true }),
    setUnreadBadge: async () => ({ ok: true }),
    copyText: async () => ({ ok: true }),
    initializeClientState: async () => ({ state: 'ready' }),
    // The mock bridge does not maintain an active socket; connection loss
    // errors are queried directly via `takeAgentConnectionLoss`.
    onAgentConnectionLoss: async () => () => {},
    onDropHover: async (listener) => {
      hoverListeners.add(listener);
      return () => hoverListeners.delete(listener);
    },
    onDropPaths: async (listener) => {
      pathListeners.add(listener);
      return () => pathListeners.delete(listener);
    },
    onWindowState: async () => () => {},
    onChatNotification: async () => () => {},
    onOpenSettings: async () => () => {},
    onNewItem: async () => () => {},
    onLockMenu: async () => () => {},
    onMaintenanceStatus: async () => () => {},
    onExitState: async () => () => {},
  };
}
