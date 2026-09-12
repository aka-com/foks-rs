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
