/** Local lifecycle/integrity outcomes. No bridge, React or network dependency. */
export const cancelled = () =>
  Object.assign(new Error('Conversation closed.'), {
    code: 'cancelled',
    message: 'Conversation closed.',
    retryable: false,
    fatal: false,
    ambiguous: false,
  });
export const integrity = (message = 'The chat identity changed.') => ({
  code: 'chat-integrity',
  message,
  retryable: false,
  fatal: true,
  ambiguous: false,
});
export const channelIntegrity = (
  message = 'Channel content could not be verified.',
) => ({
  code: 'chat-channel-integrity',
  message,
  retryable: false,
  fatal: true,
  ambiguous: false,
});
/**
 * A local access guard found that the shell's availability or access
 * generation changed around a request. The code is local, not the agent's
 * `chat-access-denied`, so the inbox does not revalidate the team for it. A
 * refusal before the request is sent can be retried; one after a mutation was
 * sent leaves its outcome unknown.
 */
export const accessChanged = (message: string, phase: 'before' | 'after') => ({
  code: 'access-changed',
  message,
  retryable: phase === 'before',
  fatal: false,
  ambiguous: phase === 'after',
  origin: 'local' as const,
});
