export class CatalogReadGate {
  private profiles = new Set<Promise<unknown>>();
  private full: Promise<unknown> | null = null;

  async profile<T>(read: () => Promise<T>): Promise<T> {
    while (this.full) await this.full.catch(() => undefined);
    const pending = Promise.resolve().then(read);
    this.profiles.add(pending);
    try {
      return await pending;
    } finally {
      this.profiles.delete(pending);
    }
  }

  exclusive<T>(read: () => Promise<T>): Promise<T> {
    const previous = this.full;
    const pending = Promise.resolve(previous)
      .catch(() => undefined)
      .then(() => Promise.allSettled([...this.profiles]))
      .then(read)
      .finally(() => {
        if (this.full === pending) this.full = null;
      });
    this.full = pending;
    return pending;
  }
}
