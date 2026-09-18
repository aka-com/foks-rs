import { decodeAcceptanceScene } from './acceptance-codec';
import { decodeProductionScene } from './scene-codec';
import type { Scene } from './types';

export function decodeForRuntimeScene(
  search: string,
  options: { fixtures: boolean },
): Scene {
  return options.fixtures
    ? decodeAcceptanceScene(search)
    : decodeProductionScene(search);
}
