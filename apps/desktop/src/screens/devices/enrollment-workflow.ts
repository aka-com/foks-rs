import type { CreateYubiAccountRequest, ProvisionYubiDeviceRequest, YubiCommand } from '../../bridge';

export function createEnrollmentCommand(
  draft: Omit<CreateYubiAccountRequest, 'email'>,
): YubiCommand {
  return {
    command: 'create_yubi_account',
    args: {
      ...draft,
      alias: draft.alias.trim(),
      username: draft.username.trim(),
      deviceName: draft.deviceName.trim(),
      email: '',
    },
  };
}

export function provisionEnrollmentCommand(
  draft: Omit<ProvisionYubiDeviceRequest, 'signingSlot' | 'pqSlot' | 'pinAttempts' | 'pukAttempts'>,
): YubiCommand {
  return {
    command: 'provision_yubi_device',
    args: {
      ...draft,
      targetAlias: draft.targetAlias.trim(),
      deviceName: draft.deviceName.trim(),
      signingSlot: 0x82,
      pqSlot: 0x83,
      pinAttempts: 3,
      pukAttempts: 3,
    },
  };
}

export function enrollmentSlot(value: string): number | null {
  return /^0x[0-9a-fA-F]{2}$/.test(value)
    ? Number.parseInt(value.slice(2), 16)
    : null;
}

export function validEnrollmentAttempts(pinAttempts: number, pukAttempts: number): boolean {
  return [pinAttempts, pukAttempts].every(
    (attempts) => Number.isInteger(attempts) && attempts > 0 && attempts <= 255,
  );
}
