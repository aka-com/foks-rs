import { normalizeCommandError } from '../bridge';
import type { ChatAction } from '../chat-contract';
import type { TrackedOperation } from './operations';
import { cancelled } from './errors';
import { RecoverySchedule } from './recovery-schedule';

const RECOVERY_BATCH = 16;
type RecoveryAction = ChatAction & {
  action: 'pending' | 'status' | 'operation-body';
};

/** One owner's bounded recovery pass. This never attempts delivery. */
export async function recoverPending(
  request: (action: RecoveryAction) => Promise<unknown>,
  operations: () => readonly TrackedOperation[],
  isCurrent: () => boolean,
  schedule = new RecoverySchedule(),
  isBlocked: (channel: string) => boolean = () => false,
): Promise<void> {
  const checkOwner = () => {
    if (!isCurrent()) throw cancelled();
  };
  const run = async (action: RecoveryAction) => {
    checkOwner();
    await request(action);
    checkOwner();
  };
  const quarantined = new Set<string>();
  await run({ action: 'pending' });
  for (const lane of ['status', 'body'] as const) {
    const visited = new Set<string>();
    for (let count = 0; count < RECOVERY_BATCH; count++) {
      checkOwner();
      const op = schedule.select(
        lane,
        operations(),
        visited,
        (channel) => quarantined.has(channel) || isBlocked(channel),
      );
      if (!op) break;
      visited.add(op.id);
      let failed = false;
      try {
        await run(
          lane === 'status'
            ? { action: 'status', operation: op.id }
            : {
                action: 'operation-body',
                operation: op.id,
                channel: op.channel,
              },
        );
      } catch (cause) {
        checkOwner();
        const error = normalizeCommandError(cause);
        if (
          error.code === 'chat-channel-integrity' ||
          error.code === 'chat-access-denied'
        ) {
          quarantined.add(op.channel);
        } else if (error.fatal || error.code === 'cancelled') {
          throw cause;
        }
        failed = true;
      }
      schedule.complete(lane, op.id, failed);
    }
  }
}
