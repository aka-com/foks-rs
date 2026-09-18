export function record(value: unknown, at: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${at} must be an object`);
  }
  return value as Record<string, unknown>;
}

export function string(value: unknown, at: string): string {
  if (typeof value !== 'string') throw new Error(`${at} must be a string`);
  return value;
}

export function entityId(
  value: unknown,
  prefix: '01' | '02' | '03' | '04' | '0d' | '10' | '14',
  at: string,
): string {
  const result = string(value, at);
  if (!new RegExp(`^${prefix}[0-9a-f]{64}$`).test(result)) {
    throw new Error(`${at} must be a canonical ${prefix} entity id`);
  }
  return result;
}

export function deviceMemberId(value: unknown, at: string): string {
  const result = string(value, at);
  if (!/^04[0-9a-f]{64}$/.test(result) && !/^08[0-9a-f]{66}$/.test(result)) {
    throw new Error(`${at} must be a canonical software-device or YubiKey id`);
  }
  return result;
}

export function bool(value: unknown, at: string): boolean {
  if (typeof value !== 'boolean') throw new Error(`${at} must be a boolean`);
  return value;
}

export function integer(value: unknown, at: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new Error(`${at} must be a non-negative safe integer`);
  }
  return value as number;
}

export function signed16(value: unknown, at: string): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < -32768 ||
    (value as number) > 32767
  ) {
    throw new Error(`${at} must be a signed 16-bit integer`);
  }
  return value as number;
}

export function array<T>(
  value: unknown,
  at: string,
  decode: (entry: unknown, at: string) => T,
): T[] {
  if (!Array.isArray(value)) throw new Error(`${at} must be an array`);
  return value.map((entry, index) => decode(entry, `${at}[${index}]`));
}

export function nullableString(value: unknown, at: string): string | null {
  return value === null ? null : string(value, at);
}

export function nullableInteger(value: unknown, at: string): number | null {
  return value === null ? null : integer(value, at);
}

export function optionalString(value: unknown, at: string): string | undefined {
  return value === undefined ? undefined : string(value, at);
}

export function optionalInteger(
  value: unknown,
  at: string,
): number | undefined {
  return value === undefined ? undefined : integer(value, at);
}

export function nullable<T>(
  value: unknown,
  at: string,
  decode: (value: unknown, at: string) => T,
): T | null {
  return value === null ? null : decode(value, at);
}
