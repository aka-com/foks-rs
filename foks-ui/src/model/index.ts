/**
 * The FOKS shell's pure model.
 *
 * TypeScript only, on purpose: the kind rule and the reader computation are
 * client-side readings with no protocol meaning, and a second copy in Rust
 * would be a second truth. Everything here is a function of the `World` it is
 * handed — no module globals, no DOM, no bridge.
 */

export * from './types';
export * from './roles';
export * from './kinds';
export * from './readers';
export * from './format';
export * from './lease';
export * from './order';
