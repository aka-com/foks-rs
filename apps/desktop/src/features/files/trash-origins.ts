/**
 * Original locations of hashed Trash entries. A name past the 255-byte limit
 * keeps the original path in a small `.origin` file beside the entry, which
 * is read once per file version while a Trash list or sheet is shown.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import type { Bridge } from '../../bridge';
import { originPath, type Item, type TrashEntry } from '../../model';

function originKey(origin: Item): string {
  return `${origin.store}|${origin.direntId}|${origin.version}`;
}

/**
 * `entries` with the original location of each hashed entry filled in from
 * its `.origin` file once read. An unreadable file leaves `original` null.
 */
export function useTrashOrigins(
  bridge: Bridge,
  entries: readonly TrashEntry[],
): readonly TrashEntry[] {
  const [paths, setPaths] = useState<ReadonlyMap<string, string | null>>(
    () => new Map(),
  );
  const requested = useRef(new Set<string>());
  useEffect(() => {
    for (const entry of entries) {
      const origin = entry.original === null ? entry.origin : undefined;
      if (!origin) continue;
      const key = originKey(origin);
      if (requested.current.has(key)) continue;
      requested.current.add(key);
      void bridge
        .readItem({
          storeId: origin.store,
          path: origin.path,
          version: origin.version,
        })
        .then(
          (read) => originPath(read.value),
          () => null,
        )
        .then((path) => setPaths((current) => new Map(current).set(key, path)));
    }
  }, [bridge, entries]);
  return useMemo(
    () =>
      entries.map((entry) => {
        const path =
          entry.original === null && entry.origin
            ? paths.get(originKey(entry.origin))
            : undefined;
        return path ? { ...entry, original: path } : entry;
      }),
    [entries, paths],
  );
}
