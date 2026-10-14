/**
 * The time source the chat services schedule on, in milliseconds. The
 * unlocked shell supplies its lease clock so that the services and the Chat
 * tab agree on when a check-in expires; tests supply a manual clock.
 */
export interface ChatClock {
  now(): number;
  later(fn: () => void, delay: number): unknown;
  cancel(timer: unknown): void;
  random(): number;
}
export const systemChatClock: ChatClock = {
  now: () => Date.now(),
  later: (fn, delay) => setTimeout(fn, delay),
  cancel: (timer) => clearTimeout(timer as ReturnType<typeof setTimeout>),
  random: Math.random,
};
