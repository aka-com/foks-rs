import { normalizeCommandError } from '../bridge';
import { chatErrorText } from './error-copy';
/** What a failed chat request says to the reader (`chatErrorText`). */
export function failure(error: unknown): string {
  return chatErrorText(normalizeCommandError(error));
}
export function preparationCanChange(error: unknown): boolean {
  return [
    'invalid-request',
    'chat-invalid-input',
    'chat-name-conflict',
    'chat-access-denied',
    'chat-unsupported',
    'chat-limit',
  ].includes(normalizeCommandError(error).code);
}
export function submissionId(): string {
  return crypto.randomUUID().replaceAll('-', '');
}
