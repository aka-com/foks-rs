import type { CommandError } from '../bridge';

/**
 * What a chat failure says to the reader. The agent's message for a `chat-*`
 * code describes the failing check in client-library terms ("unsupported chat
 * operation: latest message has unsupported content"), so a failure that came
 * from the agent is shown with the copy below instead. The agent's text stays
 * on the error object and its code in the diagnostic timing log. Failures this
 * desktop raised itself carry no `origin` and keep their own message.
 */
const CHAT_ERROR_COPY: Readonly<Record<string, string>> = {
  'chat-invalid-input':
    'Chat refused the request as invalid. Check the message or channel name and try again.',
  'chat-unsupported':
    'This team or channel uses a chat feature this version of FOKS does not support.',
  'chat-access-denied':
    'Your current role does not allow this. Access to the team is checked again.',
  'chat-refresh-required':
    'Chat state changed on the server. Try again once the channel list refreshes.',
  'chat-reprepare-required':
    'This saved message can no longer be sent as prepared. Cancel it and write it again.',
  'chat-not-found': 'The channel or saved message no longer exists.',
  'chat-key-unavailable':
    'The channel key is not available on this device yet. Try again once the team refreshes.',
  'chat-limit': 'The request exceeds a chat limit.',
  'chat-operation-state':
    'The saved message changed state. Its current status is shown in the conversation.',
  'chat-name-conflict': 'The team already has a channel with that name.',
  'chat-randomness':
    'This device could not generate a random identifier. Try again.',
  'chat-integrity': 'Chat data for this account could not be verified.',
  'chat-channel-integrity': 'Content in this channel could not be verified.',
};

export function chatErrorText(
  error: Pick<CommandError, 'code' | 'message' | 'origin'>,
): string {
  const copy = CHAT_ERROR_COPY[error.code];
  return copy !== undefined &&
    (error.origin === 'invoke' || error.origin === 'response')
    ? copy
    : error.message;
}
