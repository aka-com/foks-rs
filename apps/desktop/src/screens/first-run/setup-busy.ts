/**
 * The setup work in progress, named by its owner, or null while setup is idle.
 * `account` covers provisioning, protection and backup writes; `phrase` is the
 * recovery phrase preparation, which reads rather than writes.
 */
export type SetupBusy = 'account' | 'discovery' | 'server-check' | 'phrase';

/**
 * Collapses the workflows' busy flags into one owner. A write outranks a
 * server check, and a server check outranks phrase preparation, so a caller
 * that asks whether a write is out never sees it hidden behind a read.
 */
export function setupBusy(sources: {
  account: boolean;
  discovery: boolean;
  serverCheck: boolean;
  phrase: boolean;
}): SetupBusy | null {
  if (sources.account) return 'account';
  if (sources.discovery) return 'discovery';
  if (sources.serverCheck) return 'server-check';
  if (sources.phrase) return 'phrase';
  return null;
}

/** Whether a setup mutation or server check is out. */
export function setupMutating(busy: SetupBusy | null): boolean {
  return busy !== null && busy !== 'phrase';
}

/**
 * Whether leaving setup is refused: a write is out that this screen is the
 * only report of. A server check can be abandoned, and an acknowledged
 * provisioning is what Finish later leaves behind.
 */
export function setupLeaveBlocked(
  busy: SetupBusy | null,
  provisioning: boolean,
): boolean {
  return (busy === 'account' || busy === 'discovery') && !provisioning;
}
