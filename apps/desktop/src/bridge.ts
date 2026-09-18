/**
 * The only seam between the FOKS webview and the local agent.
 *
 * Tauri's generic `invoke<T>` does not validate types at runtime. Native
 * responses pass through runtime decoders before entering the UI state.
 */

export * from './bridge/index';
