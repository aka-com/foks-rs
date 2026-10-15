import { createBridge } from './create';
import { tauriTransport } from './transport';

export const tauriBridge = createBridge(tauriTransport);
