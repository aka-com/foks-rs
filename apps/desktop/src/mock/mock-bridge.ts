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
import { mockAccountCommands } from './accounts';
import { mockCoreCommands } from './core';
import { mockEnrollmentCommands } from './enrollment';
import { FIXTURE } from './fixture';
import { mockHost } from './host';
import { mockServerCommands } from './servers';
import { mockState } from './state';
import { mockVaultCommands } from './vault';
import { mockYubiCommands } from './yubikey';
import type { AgentSnapshot } from '../model/types';

export { mockAppLifecycleRequests, type AppLifecycleRequest } from './core';
export {
  mockCopies,
  mockOpenedSites,
  type MockCopy,
  type MockOpenedSite,
} from './vault';

/**
 * The bridge members the mock implements directly instead of through the mock
 * host: every member outside the command factories whose commands the host
 * answers. Each key is required, including the optional native-only methods,
 * which the mock sets to `undefined`. A member added to any other factory
 * therefore fails the build until the mock supplies it, rather than reaching
 * the mock host and rejecting at runtime.
 */
type MockOverrides = {
  [
    K in Exclude<keyof Bridge, keyof ReturnType<typeof vaultCommands>>
  ]: Bridge[K];
};

export function mockBridge(snapshot: AgentSnapshot = FIXTURE): Bridge {
  const state = mockState(snapshot);
  // The vault commands answer at the wire level, through the bridge's own
  // command factories, decoders and failure handling.
  return createBridge(createTransport(mockHost(mockVaultCommands(state))), {
    ...mockAccountCommands(state),
    ...mockCoreCommands(state),
    ...mockEnrollmentCommands(state),
    ...mockServerCommands(state),
    ...mockYubiCommands(state),
    native: false,
    fixtureSnapshot: snapshot,
    firstRunFixture: state.firstRunFixture,
    invitation: mockInvitations(state.stores),
  } satisfies MockOverrides);
}
