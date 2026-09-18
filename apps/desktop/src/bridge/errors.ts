import {
  bool,
  optionalInteger,
  optionalString,
  record,
  string,
} from './validation';

export interface CommandError {
  code: string;
  message: string;
  retryable: boolean;
  ambiguous: boolean;
  fatal: boolean;
  origin?: 'local' | 'invoke' | 'response';
  details?: {
    kind?: string;
    operation?: string;
    capability?: string;
    profile?: string;
    stateDir?: string;
    reason?: string;
    foundSchema?: number;
    supportedSchema?: number;
  };
}

export type CommandRecovery =
  | { kind: 'ignore' | 'retry' | 'refresh'; scope: 'request' }
  | { kind: 'reconnect'; scope: 'agent' }
  | { kind: 'quarantine'; scope: 'agent' | 'profile' | 'account' | 'channel' }
  | { kind: 'revalidate'; scope: 'team' };

export function commandRecovery(error: CommandError): CommandRecovery {
  if (
    !error.ambiguous &&
    ['cancelled', 'catalog-read-retired', 'agent-request-retired'].includes(
      error.code,
    )
  )
    return { kind: 'ignore', scope: 'request' };
  if (['agent-lost', 'bootstrap-required'].includes(error.code))
    return { kind: 'reconnect', scope: 'agent' };
  if (
    [
      'unsafe-socket',
      'protocol',
      'response-binding',
      'version-mismatch',
    ].includes(error.code)
  )
    return { kind: 'quarantine', scope: 'agent' };
  if (error.code === 'chat-integrity')
    return { kind: 'quarantine', scope: 'account' };
  if (error.code === 'chat-channel-integrity')
    return { kind: 'quarantine', scope: 'channel' };
  if (
    [
      'unsupported-schema',
      'rollback-detected',
      'checkpoint-reset-required',
      'server-identity-rejected',
      'saved-trust-missing',
    ].includes(error.code)
  )
    return { kind: 'quarantine', scope: 'profile' };
  if (
    ['chat-access-denied', 'capability-denied', 'chat-unsupported'].includes(
      error.code,
    )
  )
    return { kind: 'revalidate', scope: 'team' };
  return { kind: error.retryable ? 'retry' : 'refresh', scope: 'request' };
}

export function isAgentSessionError(error: CommandError): boolean {
  return commandRecovery(error).scope === 'agent';
}

export function normalizeMutationError(value: unknown): CommandError {
  const error = normalizeCommandError(value);
  return error.details?.reason !== 'admission-not-started' &&
    ['invalid-command-error', 'invalid-response', 'unknown'].includes(
      error.code,
    )
    ? { ...error, ambiguous: true, retryable: false }
    : error;
}

type AgentReadinessListener = (error: CommandError) => void;
const readinessListeners = new Set<AgentReadinessListener>();
const reportedReadinessErrors = new WeakSet<object>();

export function onAgentReadinessRequired(
  listener: AgentReadinessListener,
): () => void {
  readinessListeners.add(listener);
  return () => readinessListeners.delete(listener);
}

export function isAgentReadinessError(error: CommandError): boolean {
  return (
    error.code === 'bootstrap-required' ||
    error.code === 'agent-lost' ||
    error.code === 'version-mismatch'
  );
}

export function reportReadinessError(error: CommandError): void {
  if (!isAgentSessionError(error)) return;
  if (reportedReadinessErrors.has(error)) return;
  reportedReadinessErrors.add(error);
  for (const listener of readinessListeners) listener(error);
}

function decodeErrorDetails(
  value: unknown,
  at: string,
): CommandError['details'] {
  if (value === undefined) return undefined;
  const item = record(value, at);
  return {
    kind: optionalString(item.kind, `${at}.kind`),
    operation: optionalString(item.operation, `${at}.operation`),
    capability: optionalString(item.capability, `${at}.capability`),
    profile: optionalString(item.profile, `${at}.profile`),
    stateDir: optionalString(item.stateDir, `${at}.stateDir`),
    reason: optionalString(item.reason, `${at}.reason`),
    foundSchema: optionalInteger(item.foundSchema, `${at}.foundSchema`),
    supportedSchema: optionalInteger(
      item.supportedSchema,
      `${at}.supportedSchema`,
    ),
  };
}

export function decodeCommandError(value: unknown, at: string): CommandError {
  const item = record(value, at);
  const details = decodeErrorDetails(item.details, `${at}.details`);
  return {
    code: string(item.code, `${at}.code`),
    message: string(item.message, `${at}.message`),
    retryable: bool(item.retryable, `${at}.retryable`),
    ambiguous: bool(item.ambiguous, `${at}.ambiguous`),
    fatal: bool(item.fatal, `${at}.fatal`),
    ...(item.origin === 'local' ||
    item.origin === 'invoke' ||
    item.origin === 'response'
      ? { origin: item.origin }
      : {}),
    ...(details ? { details } : {}),
  };
}

export function normalizeCommandError(value: unknown): CommandError {
  if (
    typeof value === 'object' &&
    value !== null &&
    reportedReadinessErrors.has(value)
  )
    return value as CommandError;
  try {
    return decodeCommandError(value, 'command error');
  } catch {
    return {
      code: 'invalid-command-error',
      message:
        value instanceof Error
          ? value.message
          : 'An unexpected error occurred.',
      retryable: false,
      ambiguous: false,
      fatal: false,
      origin: 'local',
    };
  }
}

export function isHardStateSchemaFailure(error: CommandError): boolean {
  return error.code === 'unsupported-schema';
}

export function shouldReportPassiveServerStatusError(error: unknown): boolean {
  return !isHardStateSchemaFailure(normalizeCommandError(error));
}
