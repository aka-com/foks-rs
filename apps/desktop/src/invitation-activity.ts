/**
 * The window event the invitation panel dispatches after an action that can
 * change an account's invitations or membership requests, with the account
 * in its detail. Readers of the shared invitation rows invalidate on it. It
 * lives apart from the panel so query modules can name it without importing
 * a component.
 */
export const INVITATION_ACTIVITY = 'foks:invitation-activity';

export interface InvitationActivityDetail {
  profile?: string;
  account?: string;
}
