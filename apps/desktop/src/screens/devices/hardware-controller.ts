import type { Bridge } from '../../bridge';
import type { useWorkflowAccess } from '../../workflow-context';
import { attemptRead } from '../../commands/command-policy';

export async function refreshConnectedCards(
  bridge: Bridge,
  access: ReturnType<typeof useWorkflowAccess>,
  profile: string,
  isCurrent: () => boolean,
  onCards: (cards: { serial: number }[]) => void,
  onError: (error: unknown) => void,
): Promise<void> {
  const result = await attemptRead({ kind: 'read-recovery' }, () =>
    access.run('yubi-scan', { profile }, () => bridge.listYubiCards(profile)),
  );
  if (!isCurrent()) return;
  if (result.outcome === 'read') onCards(result.value);
  else onError(result.error);
}
