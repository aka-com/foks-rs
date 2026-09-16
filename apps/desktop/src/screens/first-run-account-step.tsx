import { Inset, InsetRow } from '../components';

/** Account methods share drafts; a connected account is read-only. */
export function AccountFields({
  username,
  deviceName,
  disabled,
  onUsername,
  onDeviceName,
}: {
  username: string;
  deviceName: string;
  disabled: boolean;
  onUsername: (value: string) => void;
  onDeviceName: (value: string) => void;
}) {
  return (
    <Inset className="account-form">
      <InsetRow label="Username">
        <input
          value={username}
          placeholder="yourname"
          disabled={disabled}
          onChange={(event) => onUsername(event.target.value)}
        />
      </InsetRow>
      <InsetRow label="This device’s name">
        <input
          value={deviceName}
          placeholder="Your device"
          disabled={disabled}
          onChange={(event) => onDeviceName(event.target.value)}
        />
      </InsetRow>
    </Inset>
  );
}
