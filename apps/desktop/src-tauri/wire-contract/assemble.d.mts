export interface FixtureInventory {
  sourceOfTruth: string;
  assembleCommand: string;
  checkCommand: string;
  generation: { status: string; reason: string; scope: string };
  domains: Record<string, string[]>;
  representedShapes: Record<string, string[]>;
  uncoveredPublicShapes: Record<string, string[]>;
  partialCoverage: Record<string, string[]>;
}

export function parseUniqueJson(text: string, source?: string): unknown;
export function mergeDomains(
  domains: Record<string, Record<string, unknown>>,
): Record<string, unknown>;
export function loadDomains(directory?: URL): Promise<{
  inventory: FixtureInventory;
  domains: Record<string, Record<string, unknown>>;
  aggregate: Record<string, unknown>;
}>;
