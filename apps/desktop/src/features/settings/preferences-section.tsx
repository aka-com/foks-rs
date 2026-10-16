/**
 * Settings › Preferences: appearance, desktop alerts, automatic Trash
 * emptying, quit confirmation and onboarding tips, kept on this device.
 */

import { useDismissedOnboardingTip } from '../../onboarding-tips';
import {
  rememberTrashAutoEmpty,
  storedTrashAutoEmpty,
  TRASH_AUTO_EMPTY_CHOICES,
} from '../../trash-auto-empty';
import {
  setAppearance,
  useAppearance,
  type Appearance,
} from '../../platform/appearance';
import {
  rememberQuitConfirmation,
  useQuitConfirmation,
} from '../../quit-confirmation';
import { useState } from 'react';
import type { ReactNode } from 'react';
import { Inset, InsetRow, SectionLabel } from '../../components';
import {
  RAIL_COLORS,
  applyRailColor,
  rememberRailColor,
  storedRailColor,
} from '../../rail-theme';
import { NotificationSettings } from '../../chat/notification-provider';

/** Locally stored desktop alert and appearance preferences. */
export function PreferencesSection(): ReactNode {
  const [passwordTipDismissed, setPasswordTipDismissed] =
    useDismissedOnboardingTip('add-password');
  const [checklistDismissed, setChecklistDismissed] =
    useDismissedOnboardingTip('setup-checklist');
  return (
    <>
      <SectionLabel>Appearance</SectionLabel>
      <RailColorPicker />
      <SectionLabel>Desktop alerts</SectionLabel>
      <NotificationSettings />
      <SectionLabel>Trash</SectionLabel>
      <TrashAutoEmptyPicker />
      <SectionLabel>Quitting</SectionLabel>
      <QuitConfirmationPicker />
      <SectionLabel>Onboarding tips</SectionLabel>
      <Inset className="settings-checkboxes">
        <label>
          <input
            type="checkbox"
            aria-label="Show the add a password tip"
            checked={!passwordTipDismissed}
            onChange={(event) => setPasswordTipDismissed(!event.target.checked)}
          />
          <span>Add a password</span>
        </label>
        <label>
          <input
            type="checkbox"
            aria-label="Show the Get set up checklist"
            checked={!checklistDismissed}
            onChange={(event) => setChecklistDismissed(!event.target.checked)}
          />
          <span>Get set up checklist, in the header’s attention list</span>
        </label>
      </Inset>
    </>
  );
}

/**
 * Automatic emptying of vault Trash on this device. While this app runs,
 * entries deleted longer ago than the chosen age are deleted permanently,
 * for everyone, from each vault's Trash where this account can change them.
 */
function TrashAutoEmptyPicker(): ReactNode {
  const [days, setDays] = useState(storedTrashAutoEmpty);
  return (
    <>
      <Inset className="settings-inset middle wide">
        <InsetRow label="Empty automatically">
          <div
            className="seg txt"
            role="group"
            aria-label="Empty Trash automatically"
          >
            {TRASH_AUTO_EMPTY_CHOICES.map((choice) => (
              <button
                type="button"
                key={choice ?? 'never'}
                className={days === choice ? 'on' : undefined}
                aria-pressed={days === choice}
                onClick={() => {
                  setDays(choice);
                  rememberTrashAutoEmpty(choice);
                }}
              >
                {choice === null ? 'Never' : `After ${choice} days`}
              </button>
            ))}
          </div>
        </InsetRow>
      </Inset>
      <p className="hint">
        Entries that have been in Trash this long are deleted permanently, for
        everyone, while this app runs. The choice applies to this device only.
      </p>
    </>
  );
}

/**
 * When closing the window or quitting FOKS asks before it stops the agent.
 * The native close guard applies the choice; see `src/quit-confirmation.ts`.
 */
function QuitConfirmationPicker(): ReactNode {
  const confirmation = useQuitConfirmation();
  return (
    <>
      <Inset className="settings-inset middle wide">
        <InsetRow label="Ask before quitting">
          <div
            className="seg txt"
            role="group"
            aria-label="Ask before quitting"
          >
            {(['always', 'when-pending'] as const).map((choice) => (
              <button
                type="button"
                key={choice}
                className={confirmation === choice ? 'on' : undefined}
                aria-pressed={confirmation === choice}
                onClick={() => rememberQuitConfirmation(choice)}
              >
                {choice === 'always' ? 'Always' : 'When work is pending'}
              </button>
            ))}
          </div>
        </InsetRow>
      </Inset>
      <p className="hint">
        Quitting FOKS always stops its agent. With When work is pending, FOKS
        asks first only when messages are unsent, a change or data maintenance
        is in progress, or FOKS cannot confirm that this app started the agent.
        The choice applies to this device only.
      </p>
    </>
  );
}

/**
 * The rail color picker. Applies each choice at once, so the rail beside
 * this page shows it.
 */
function RailColorPicker(): ReactNode {
  const appearance = useAppearance();
  const [color, setColor] = useState(storedRailColor);
  const choose = (id: string): void => {
    setColor(id);
    applyRailColor(id);
    rememberRailColor(id);
  };
  return (
    <Inset className="settings-inset middle wide">
      <InsetRow label="Theme">
        <div
          className="seg txt appearance-picker"
          role="group"
          aria-label="Theme"
        >
          {(['light', 'dark', 'system'] as const).map((value: Appearance) => (
            <button
              type="button"
              key={value}
              className={appearance === value ? 'on' : undefined}
              aria-pressed={appearance === value}
              onClick={() => setAppearance(value)}
            >
              {value === 'light'
                ? 'Light'
                : value === 'dark'
                  ? 'Dark'
                  : 'System'}
            </button>
          ))}
        </div>
      </InsetRow>
      <InsetRow label="Sidebar color">
        <div className="swatches" role="radiogroup" aria-label="Sidebar color">
          {RAIL_COLORS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              role="radio"
              className={entry.id === color ? 'swatch on' : 'swatch'}
              aria-checked={entry.id === color}
              aria-label={entry.label}
              title={`${entry.label} — light / dark`}
              onClick={() => choose(entry.id)}
            >
              <i
                style={{
                  background: `linear-gradient(135deg, ${entry.hex} 50%, ${entry.darkHex} 50%)`,
                }}
                aria-hidden="true"
              />
              <span>{entry.label}</span>
            </button>
          ))}
        </div>
      </InsetRow>
    </Inset>
  );
}
