import type { Bridge } from '../bridge';

export interface PaperKeyDraft {
  phrase: string;
  alias: string;
}
interface Reveal extends PaperKeyDraft {
  hidden: boolean;
  timer?: ReturnType<typeof setTimeout>;
}

/** Front-end memory only. A concealed reveal can be taken once, for two minutes. */
export class PaperKeyResume {
  private reveal: Reveal | null = null;
  private listeners = new Set<() => void>();
  get = (): Reveal | null => this.reveal;
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  private publish(): void {
    for (const listener of this.listeners) listener();
  }
  forget = (): void => {
    if (this.reveal?.timer) clearTimeout(this.reveal.timer);
    this.reveal = null;
    this.publish();
  };
  prepare = (draft: PaperKeyDraft, concealed = false): void => {
    this.forget();
    this.reveal = { ...draft, hidden: false };
    if (concealed) this.conceal();
    else this.publish();
  };
  conceal = (): void => {
    if (!this.reveal || this.reveal.hidden) return;
    this.reveal = {
      ...this.reveal,
      hidden: true,
      timer: setTimeout(this.forget, 120_000),
    };
    this.publish();
  };
  take = (): PaperKeyDraft | null => {
    const draft = this.reveal;
    this.forget();
    return draft ? { phrase: draft.phrase, alias: draft.alias } : null;
  };
}

const sessions = new WeakMap<Bridge, Map<string, PaperKeyResume>>();
export function paperKeyResume(
  bridge: Bridge,
  account: string,
): PaperKeyResume {
  let accounts = sessions.get(bridge);
  if (!accounts) {
    accounts = new Map();
    sessions.set(bridge, accounts);
  }
  let resume = accounts.get(account);
  if (!resume) {
    resume = new PaperKeyResume();
    accounts.set(account, resume);
  }
  return resume;
}
