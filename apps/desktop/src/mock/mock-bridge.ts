/**
 * A deterministic command bridge for render and browser acceptance tests.
 * Each domain's commands are in their own module, named after the
 * `bridge/commands-*.ts` factory whose members it supplies, over the records
 * in `state.ts`.
 */

import { mockInvitations } from './invitation-mock';
import type { Bridge } from '../bridge';
import type { vaultCommands } from '../bridge/commands-vault';
import { createBridge } from '../bridge/create';
import { createTransport } from '../bridge/transport';
import {
  ACCOUNT_HOST_MEMBERS,
  mockAccountCommands,
  mockAccountHost,
} from './accounts';
import { mockCoreCommands } from './core';
import {
  ENROLLMENT_HOST_MEMBERS,
  mockEnrollmentCommands,
  mockEnrollmentHost,
} from './enrollment';
import { FIXTURE } from './fixture';
import { mockHost, type MockCommand } from './host';
import {
  mockServerCommands,
  mockServerHost,
  SERVER_HOST_MEMBERS,
} from './servers';
import { mockState, type MockState } from './state';
import { mockVaultCommands } from './vault';
import { mockYubiHost, YUBI_HOST_MEMBERS } from './yubikey';
import type { AgentSnapshot } from '../model/types';

export { mockAppLifecycleRequests, type AppLifecycleRequest } from './core';
export {
  mockCopies,
  mockOpenedSites,
  type MockCopy,
  type MockOpenedSite,
} from './vault';

/**
 * The members outside the vault factory whose commands the mock host answers
 * at the wire level. Every member of the vault factory answers there too.
 */
export const MOCK_HOST_MEMBERS = [
  ...ACCOUNT_HOST_MEMBERS,
  ...ENROLLMENT_HOST_MEMBERS,
  ...SERVER_HOST_MEMBERS,
  ...YUBI_HOST_MEMBERS,
] as const;

/**
 * The bridge members the mock implements directly instead of through the mock
 * host: every member outside the vault factory and `MOCK_HOST_MEMBERS`. Each
 * key is required, including the optional native-only methods, which the mock
 * sets to `undefined`. A member added to a factory therefore fails the build
 * until the mock supplies it, rather than reaching the mock host and
 * rejecting at runtime.
 */
type MockOverrides = {
  [
    K in Exclude<
      keyof Bridge,
      | keyof ReturnType<typeof vaultCommands>
      | (typeof MOCK_HOST_MEMBERS)[number]
    >
  ]: Bridge[K];
};

/** The commands the mock host answers, by native command name. */
export function mockHostCommands(
  state: MockState,
): Record<string, MockCommand> {
  return {
    ...mockVaultCommands(state),
    ...mockAccountHost(state),
    ...mockEnrollmentHost(state),
    ...mockServerHost(state),
    ...mockYubiHost(state),
  };
}

export function mockBridge(snapshot: AgentSnapshot = FIXTURE): Bridge {
  const state = mockState(snapshot);
  // The commands the host answers go through the bridge's own command
  // factories, decoders and failure handling.
  return createBridge(createTransport(mockHost(mockHostCommands(state))), {
    ...mockAccountCommands(state),
    ...mockCoreCommands(state),
    ...mockEnrollmentCommands(),
    ...mockServerCommands(state),
    native: false,
    fixtureSnapshot: snapshot,
    firstRunFixture: state.firstRunFixture,
    invitation: mockInvitations(state.stores),
  } satisfies MockOverrides);
}
