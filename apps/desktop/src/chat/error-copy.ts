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
    'This saved message can no longer be sent as prepared. Write it again.',
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

/**
 * Where a channel creation's failure differs from a message's. The agent
 * returns `chat-reprepare-required` for a creation when the team's channel set
 * or the pending channel keys changed since it was prepared.
 */
const CHANNEL_CREATION_COPY: Readonly<Record<string, string>> = {
  'chat-reprepare-required':
    "The team's channels changed since this creation was prepared. Review the name and audience, then create the channel again.",
  'chat-not-found': 'The saved channel creation no longer exists.',
  'chat-operation-state':
    'The saved channel creation changed state before this request finished.',
};

/**
 * `subject` is what the failed request was about: a message (the default) or
 * a channel creation, whose copy differs for the codes above.
 */
export function chatErrorText(
  error: Pick<CommandError, 'code' | 'message' | 'origin'>,
  subject: 'message' | 'channel-creation' = 'message',
): string {
  const copy =
    (subject === 'channel-creation'
      ? CHANNEL_CREATION_COPY[error.code]
      : undefined) ?? CHAT_ERROR_COPY[error.code];
  return copy !== undefined &&
    (error.origin === 'invoke' || error.origin === 'response')
    ? copy
    : error.message;
}
