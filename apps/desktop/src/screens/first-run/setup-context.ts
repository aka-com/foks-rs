import { createContext, useContext } from 'react';
import type { Bridge, CommandError, PendingOperation } from '../../bridge';
import type {
  FirstRunCheckpoint,
  FirstRunPath,
  FirstRunStateName,
} from '../../first-run-state';
import type { Location } from '../../location';
import type { AgentSnapshot } from '../../model';
import type { RailAgentState } from '../../shell/sidebar';
import type { SsoAction, SsoProgress } from '../../sso-contract';
import type { useFirstRunController } from '../../use-first-run-controller';
import type { SigninMethod } from './session-transitions';
import type { SetupBusy } from './setup-busy';
import type { SetupStores } from './setup-stores';
import type { useAccountOperations } from './use-account-operations';
import type { CliDiscovery } from './use-cli-discovery';
import type { useProtectionActions } from './use-protection-actions';
import type { useServerWorkflow } from './use-server-workflow';
import type { SetupIdentity } from './use-setup-identity';
import type { SetupSecrets } from './use-setup-secrets';
import type { SetupRestart } from './use-setup-session';
import type { useTeamDiscovery } from './use-team-discovery';

export interface FirstRunExperienceProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'first-run' }>;
  onNavigate: (location: Location) => void;
  onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  concealSignal: number;
  agentReady: boolean;
  onRetryAgent?: () => Promise<void>;
  onAgentReadinessFailure?: (error: CommandError) => void;
  automaticEntry?: boolean;
  managedProfile?: string;
  /** Connection status indicator in the sidebar footer during first-run setup. */
  agent?: RailAgentState;
  blocked?: boolean;
  /** The rail's width, carried in and out of setup. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
  /** The Devices tab's dot, as the shell computes it for every other screen. */
  devicesAlert?: { description: string } | null;
}

/** One setup session's props: the shell's, and the session it replaces itself with. */
export type FirstRunSessionProps = FirstRunExperienceProps & {
  onReplaceSession: (next: FirstRunCheckpoint) => void;
  sessionEntry: FirstRunCheckpoint | null;
};

type SetupController = ReturnType<typeof useFirstRunController>;
type ConnectionErrors = Record<'copy' | 'recover' | 'pair', string | null>;

/** Account provisioning: the operation record, and the commands that start or settle it. */
export type SetupAccount = ReturnType<typeof useAccountOperations> & {
  /** Per sign-in method failures, shown beside the method that failed. */
  connectionErrors: ConnectionErrors;
  create: () => Promise<void>;
  copyCandidate: () => Promise<void>;
  recover: () => Promise<void>;
  acceptPairing: (resume: boolean) => Promise<void>;
  resume: () => Promise<void>;
  discardProvisioning: () => void;
  discardProvisionedAccount: () => void;
  executeSsoSignup: (
    action: SsoAction,
    operation: () => Promise<SsoProgress>,
  ) => Promise<SsoProgress>;
};

/**
 * Everything a setup step reads or changes. useFirstRunSession builds it on
 * every render, and FirstRunSession provides it to the step the checkpoint
 * names, so a step reads what it needs here instead of through props.
 */
export interface SetupSession extends SetupController {
  readonly bridge: Bridge;
  readonly snapshot: AgentSnapshot;
  readonly agentReady: boolean;
  readonly onNavigate: (location: Location) => void;
  readonly onRefreshSnapshot: (force?: boolean) => Promise<AgentSnapshot>;
  /** The one setup operation in progress, or null while setup is idle. */
  readonly busy: SetupBusy | null;
  /** Whether a wait has run long enough to say so. */
  readonly slow: boolean;
  /** Whether Leave setup is refused because a write is out. */
  readonly leaveBlocked: boolean;
  readonly message: string | null;
  readonly setMessage: (message: string | null) => void;
  /**
   * The raw error chain behind a reported failure, offered only while the
   * text that failure reported is still the text on screen.
   */
  readonly reasonFor: (shown: string | null) => string | undefined;
  /** Moves to another step through the checkpoint machine, dropping the step's drafts. */
  readonly go: (next: FirstRunStateName) => void;
  /** The agent's pending operations for the checked server. */
  readonly pending: readonly PendingOperation[];
  readonly identity: SetupIdentity & { edit: (username: string) => void };
  readonly secrets: SetupSecrets;
  readonly cli: CliDiscovery;
  readonly server: ReturnType<typeof useServerWorkflow>;
  readonly team: ReturnType<typeof useTeamDiscovery>;
  readonly account: SetupAccount;
  readonly protection: ReturnType<typeof useProtectionActions>;
  readonly restart: SetupRestart;
  readonly stores: SetupStores;
  /** Reloading the catalog when the end steps' vaults are not readable yet. */
  readonly personal: {
    refreshing: boolean;
    error: string | null;
    retry: () => void;
    retryTeam: () => void;
  };
  /** Retrying the agent when setup is blocked on it. */
  readonly agentRetry: {
    retrying: boolean;
    error: string | null;
    retry: (() => void) | null;
  };
  /** The path picked on "How are you joining?" before it is confirmed. */
  readonly joining: {
    path: FirstRunPath | null;
    setPath: (path: FirstRunPath | null) => void;
  };
  /**
   * The sign-in method picked on the sign-in step. Not persisted: it is a
   * choice within one page, and leaving the sign-in path clears it.
   */
  readonly signin: {
    chosen: SigninMethod | null;
    setChosen: (method: SigninMethod | null) => void;
  };
  /** Organization sign-up: whether it is chosen, and where its primary action renders. */
  readonly sso: {
    shown: boolean;
    primarySlot: HTMLElement | null;
    setPrimarySlot: (slot: HTMLElement | null) => void;
  };
}

export const SetupContext = createContext<SetupSession | null>(null);

/** The setup session a step renders in. Only steps under FirstRunSession call it. */
export function useSetupContext(): SetupSession {
  const session = useContext(SetupContext);
  if (!session)
    throw new Error('A setup step rendered outside FirstRunSession.');
  return session;
}
