import {
  decodeFirstRunCheckpoint,
  encodeFirstRunCheckpoint,
  type FirstRunCheckpoint,
} from './first-run-state';

const KEY = 'foks.setup-recovery.v1';
export interface RetainedSetup {
  readonly id: string;
  readonly checkpoint: FirstRunCheckpoint;
}

/** Nonsecret references to native receipts/agent operations, not execution authority. */
export function retainedSetups(): RetainedSetup[] {
  const raw = window.localStorage.getItem(KEY);
  if (!raw) return [];
  const parsed: unknown = JSON.parse(raw);
  if (!Array.isArray(parsed))
    throw new Error('Saved setup attempts could not be read.');
  return parsed.map((entry: unknown) => {
    if (
      !entry ||
      typeof entry !== 'object' ||
      !('id' in entry) ||
      typeof entry.id !== 'string' ||
      !('checkpoint' in entry)
    )
      throw new Error('Saved setup attempts could not be read.');
    const checkpoint = decodeFirstRunCheckpoint(
      JSON.stringify(entry.checkpoint),
    );
    if (!checkpoint) throw new Error('Saved setup attempts could not be read.');
    return { id: entry.id, checkpoint };
  });
}

function save(entries: RetainedSetup[]): void {
  window.localStorage.setItem(
    KEY,
    JSON.stringify(
      entries.map((entry) => ({
        id: entry.id,
        checkpoint: JSON.parse(
          encodeFirstRunCheckpoint(entry.checkpoint),
        ) as unknown,
      })),
    ),
  );
}
export function sameSetupTarget(
  a: FirstRunCheckpoint,
  b: FirstRunCheckpoint,
): boolean {
  const alias = (value: FirstRunCheckpoint) =>
    value.provisioning?.alias ??
    value.provisionedAccount?.alias ??
    value.sso?.alias ??
    value.account?.alias;
  return Boolean(
    a.profile &&
    b.profile &&
    alias(a) &&
    alias(a) === alias(b) &&
    a.profile.profile === b.profile.profile &&
    a.profile.hostId === b.profile.hostId,
  );
}
export function retainSetup(checkpoint: FirstRunCheckpoint): void {
  if (
    !checkpoint.provisioning &&
    !checkpoint.provisionedAccount &&
    !checkpoint.sso
  )
    return;
  if (!decodeFirstRunCheckpoint(encodeFirstRunCheckpoint(checkpoint)))
    throw new Error('Setup progress could not be saved.');
  const entries = retainedSetups();
  const id =
    checkpoint.provisioning?.id ??
    checkpoint.sso?.operationId ??
    `${checkpoint.profile!.hostId}/${checkpoint.profile!.profile}/${checkpoint.provisionedAccount!.alias}`;
  save([...entries.filter((entry) => entry.id !== id), { id, checkpoint }]);
}
/** Late completion updates its retained attempt, never the new wizard. */
export function updateRetainedSetup(
  previous: FirstRunCheckpoint,
  next: FirstRunCheckpoint,
): void {
  const entries = retainedSetups();
  let changed = false;
  const updated = entries.flatMap((entry) => {
    const matches = previous.provisioning
      ? entry.checkpoint.provisioning?.id === previous.provisioning.id
      : previous.sso
        ? entry.checkpoint.sso?.operationId === previous.sso.operationId
        : sameSetupTarget(entry.checkpoint, previous);
    if (!matches) return [entry];
    changed = true;
    return next.provisioning || next.provisionedAccount || next.sso
      ? [{ ...entry, checkpoint: next }]
      : [];
  });
  if (changed) save(updated);
}
