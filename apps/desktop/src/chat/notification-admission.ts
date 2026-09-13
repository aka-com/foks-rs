/** Include cancelled-but-unsettled jobs across Strict Mode / owner replacement. */
const owners = new WeakMap<object, NotificationAdmission>();
export class NotificationAdmission {
  private profiles = new Set<string>();
  private listeners = new Set<() => void>();
  get active() {
    return this.profiles.size;
  }
  acquire(profile: string): (() => void) | undefined {
    if (this.profiles.size >= 2 || this.profiles.has(profile)) return;
    this.profiles.add(profile);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.profiles.delete(profile);
      for (const fn of this.listeners) fn();
    };
  }
  subscribe(fn: () => void) {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }
}
export function notificationAdmission(owner: object): NotificationAdmission {
  let admission = owners.get(owner);
  if (!admission) owners.set(owner, (admission = new NotificationAdmission()));
  return admission;
}
