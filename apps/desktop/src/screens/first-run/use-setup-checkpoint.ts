import type { Bridge } from '../../bridge';
import { resolveSetupEntry } from '../../first-run-controller';
import { storedFirstRunCheckpoint } from '../../first-run-operations';
import {
  initialFirstRun,
  isFirstRunState,
  type FirstRunCheckpoint,
  type FirstRunPath,
  type FirstRunStateName,
} from '../../first-run-state';
import type { Location } from '../../location';
import type { AgentSnapshot } from '../../model';
import { useFirstRunController } from '../../use-first-run-controller';
import { stepOf } from '../first-run-view';

function fixtureSeed(
  bridge: Bridge,
  path: FirstRunPath,
  state: FirstRunStateName,
): FirstRunCheckpoint {
  const facts = bridge.firstRunFixture?.[path];
  let next = initialFirstRun(path, state);
  if (state === 'boot') return next;
  if (state === 'error' && facts) next = { ...next, serverAddress: facts.typo };
  if (
    (stepOf(state) >= 2 || state === 'checked' || state === 'compare') &&
    facts
  ) {
    next = { ...next, profile: facts.report, serverAddress: facts.server };
  }
  if (stepOf(state) >= 3 && facts) {
    next = {
      ...next,
      account: {
        alias: facts.accountAlias,
        username: facts.username,
        deviceName: facts.deviceName,
      },
    };
  }
  if (state === 'identity-pending' && facts)
    next = {
      ...next,
      provisionedAccount: {
        alias: facts.accountAlias,
        deviceName: facts.deviceName,
      },
    };
  if (stepOf(state) >= 4 && state !== 'phrase')
    next = { ...next, passphraseSet: true, backupCommitted: true };
  if (state === 'added')
    next = {
      ...next,
      added: true,
      group:
        facts?.groupName && facts.groupAlias && facts.groupTeamIdHex
          ? {
              name: facts.groupName,
              kind: 'named',
              alias: facts.groupAlias,
              teamIdHex: facts.groupTeamIdHex,
            }
          : undefined,
    };
  if (state === 'checklist-invited')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      added: false,
    };
  if (state === 'checklist-own')
    next = {
      ...next,
      passphraseSet: false,
      backupCommitted: false,
      protectSkipped: true,
      group: undefined,
    };
  return next;
}

export function initialCheckpoint(
  bridge: Bridge,
  snapshot: AgentSnapshot,
  location: Extract<Location, { kind: 'first-run' }>,
  automaticEntry: boolean,
): FirstRunCheckpoint {
  const path: FirstRunPath = location.path ?? 'invited';
  const saved = storedFirstRunCheckpoint();
  const queryState =
    typeof window === 'undefined'
      ? null
      : new URLSearchParams(window.location.search).get('state');
  const namedReviewState = isFirstRunState(queryState);
  if (bridge.firstRunFixture && namedReviewState)
    return fixtureSeed(bridge, path, location.step as FirstRunStateName);
  const state =
    location.step === 'boot' || !isFirstRunState(location.step)
      ? 'who'
      : location.step;
  return resolveSetupEntry(
    snapshot,
    saved,
    { state, path: location.path },
    automaticEntry,
  );
}

/**
 * The session's checkpoint controller. It starts from the checkpoint this
 * session replaced the previous one with, or else from the entry the
 * location, the saved checkpoint and the snapshot resolve to.
 */
export function useSetupCheckpoint({
  sessionEntry,
  automaticEntry,
  ...options
}: Omit<Parameters<typeof useFirstRunController>[0], 'initial'> & {
  sessionEntry: FirstRunCheckpoint | null;
  automaticEntry: boolean;
}) {
  return useFirstRunController({
    ...options,
    initial: () =>
      sessionEntry ??
      initialCheckpoint(
        options.bridge,
        options.snapshot,
        options.location,
        automaticEntry,
      ),
  });
}
