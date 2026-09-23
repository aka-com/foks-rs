// Matches foks_proto::MAXIMUM_DEVICE_NAME_BYTES; this bounds display bytes,
// independently of the protocol's normalized 200-character limit.
export const MAXIMUM_DEVICE_NAME_BYTES = 4096;

export function boundedDeviceName(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length > 0 &&
    new TextEncoder().encode(value).length <= MAXIMUM_DEVICE_NAME_BYTES &&
    value.trim() === value &&
    [...value].every((c) => c.charCodeAt(0) > 31 && c.charCodeAt(0) !== 127)
  );
}

/** Presentation cleanup matching foks-client's fix_device_name. Native
 * validation remains authoritative; do not use compatibility normalization. */
export function fixDeviceName(name: string): string {
  return name
    .replace(/[‘’‚‛]/gu, "'")
    .replace(/[‐‑–—−]/gu, '-')
    .replace(/[Ａ-Ｚａ-ｚ０-９．＿＋＇－]/gu, (c) =>
      String.fromCodePoint(c.codePointAt(0)! - 0xfee0),
    )
    .replace(/\p{White_Space}+/gu, ' ')
    .replace(/^ +| +$/gu, '');
}
