import { normalizeCommandError } from '../bridge';
export function failure(error: unknown): string {
  return normalizeCommandError(error).message;
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
