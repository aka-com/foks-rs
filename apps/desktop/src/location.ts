/**
 * Navigation state management and URL routing.
 *
 * Provides a subscription-based external store managing application location,
 * view options, selection, and query state without direct DOM dependencies.
 * Stores and accounts are identified by canonical StoreRef identifiers.
 */

export * from './navigation/types';
export {
  accountAtLocation,
  parentLocation,
  railTabOf,
  sameLocation,
} from './navigation/routes';
export {
  chatTabLocation,
  rememberChatLocation,
  rememberedChatRef,
} from './navigation/chat-tab-memory';
export { transition } from './navigation/transition';
export { SETTINGS_SECTION_ALIASES } from './navigation/legacy-routes';
export {
  decodeProductionLocation,
  encodeLocation,
  getState,
  locationHref,
  setUrl,
} from './navigation/production-codec';
export {
  decodeProductionScene,
  sceneHref,
  sceneOf,
} from './navigation/scene-codec';
export {
  decodeAcceptanceScene,
  decodeLocation,
  decodeScene,
} from './navigation/acceptance-codec';
export { decodeForRuntimeScene } from './navigation/runtime-scene';
export { LocationStore, storeAtScene } from './navigation/location-store';
export { useLocationState } from './navigation/use-location-state';
