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
  if (!isAgentReadinessError(error)) return;
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
          : 'The local agent returned an invalid error.',
      retryable: false,
      ambiguous: true,
      fatal: true,
    };
  }
}

export function isHardStateSchemaFailure(error: CommandError): boolean {
  return error.code === 'unsupported-schema';
}

export function shouldReportPassiveServerStatusError(error: unknown): boolean {
  return !isHardStateSchemaFailure(normalizeCommandError(error));
}
