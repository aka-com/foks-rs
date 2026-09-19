import type { ReactNode } from 'react';
import { Band, Button } from '../components';
import type { Severity } from '../model';

export interface ChatAlertAction {
  label: string;
  run: () => void;
  disabled?: boolean;
}

export interface ChatAlert {
  message: string;
  severity: Severity;
  label?: string;
  actions?: readonly ChatAlertAction[];
}

const severityOrder: Record<Severity, number> = { info: 0, warn: 1, crit: 2 };

export function ChatAlerts({
  alerts,
}: {
  alerts: readonly ChatAlert[];
}): ReactNode {
  const grouped = new Map<string, ChatAlert>();
  for (const alert of alerts) {
    const key = alert.message.trim().replace(/\s+/g, ' ');
    if (!key) continue;
    const previous = grouped.get(key);
    const strongest =
      previous &&
      severityOrder[previous.severity] >= severityOrder[alert.severity]
        ? previous
        : alert;
    grouped.set(key, {
      ...strongest,
      actions: [...(previous?.actions ?? []), ...(alert.actions ?? [])],
    });
  }
  if (!grouped.size) return null;
  return (
    <div className="chat-status">
      {[...grouped].map(([key, alert]) => {
        const actions = new Map<string, ChatAlertAction[]>();
        for (const action of alert.actions ?? [])
          actions.set(action.label, [
            ...(actions.get(action.label) ?? []),
            action,
          ]);
        return (
          <Band
            key={key}
            severity={alert.severity}
            live
            action={
              actions.size
                ? [...actions].map(([label, matching]) => {
                    const enabled = matching.filter(
                      (action) => !action.disabled,
                    );
                    return (
                      <Button
                        key={label}
                        size="sm"
                        disabled={!enabled.length}
                        onClick={() => {
                          for (const run of new Set(
                            enabled.map((action) => action.run),
                          ))
                            run();
                        }}
                      >
                        {label}
                      </Button>
                    );
                  })
                : undefined
            }
          >
            {alert.label ? (
              <>
                {alert.label} <small>{alert.message}</small>
              </>
            ) : (
              alert.message
            )}
          </Band>
        );
      })}
    </div>
  );
}
