/**
 * The header's attention control: one bell button, with a count of the items
 * that need the reader, and the popover it opens.
 *
 * The popover lists, in three sections, what `attention.ts` derives from the
 * shell's facts: the items that need the reader, each a link to the page that
 * resolves it; the "Get set up" steps still open after first run; and the
 * last toast messages this session showed. The rail's counts and dots stay
 * where they are; this is the one place that names each of them. Recent holds
 * message text only, in memory, and the shell clears it when the app locks.
 */

import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Popover } from '/kit/overlay-primitives';
import type { ToastRecord } from '/kit/toasts';
import type { AttentionItem, SetupStep } from '../attention';
import { Icon } from '../components';
import type { Location } from '../location';
import { plural } from '../model';
import { useDismissedOnboardingTip } from '../onboarding-tips';

export const ATTENTION_PANEL_ID = 'attention-centre';

export interface AttentionCentreProps {
  items: readonly AttentionItem[];
  checklist: readonly SetupStep[];
  /** The toast history, oldest first. */
  recent: readonly ToastRecord[];
  onNavigate: (location: Location) => void;
  onClearRecent?: () => void;
  /** A blocking state: the button is drawn, and does not open. */
  blocked?: boolean;
}

/** The button's accessible name: the count when there is one. */
export function attentionLabel(count: number): string {
  return count
    ? `${plural(count, 'item')} ${count === 1 ? 'needs' : 'need'} attention`
    : 'Nothing needs attention';
}

function shownAt(record: ToastRecord): string {
  return new Date(record.shownAt).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });
}

export function AttentionControl({
  items,
  checklist,
  recent,
  onNavigate,
  onClearRecent,
  blocked = false,
}: AttentionCentreProps): ReactNode {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [checklistHidden, setChecklistHidden] =
    useDismissedOnboardingTip('setup-checklist');
  useEffect(() => {
    if (blocked) setOpen(false);
  }, [blocked]);
  // The panel takes the focus when it opens, so the keyboard reaches its
  // links without walking the rest of the header first.
  useEffect(() => {
    if (open) panelRef.current?.focus();
  }, [open]);
  const close = (): void => setOpen(false);
  const go = (location: Location): void => {
    close();
    onNavigate(location);
  };
  // A step this computer cannot do is listed and not counted, so it neither
  // holds the checklist open nor counts toward its progress.
  const counted = checklist.filter((step) => !step.unavailable);
  const done = counted.filter((step) => step.done).length;
  const showChecklist =
    !checklistHidden && counted.length > 0 && done < counted.length;
  const label = attentionLabel(items.length);
  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        className={`attention-trigger${items.length ? ' has-items' : ''}`}
        aria-label={label}
        title="Attention and recent activity"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? ATTENTION_PANEL_ID : undefined}
        disabled={blocked}
        onClick={() => setOpen(!open)}
      >
        <Icon name="bell" />
        {items.length ? (
          <span className="attention-count" aria-hidden="true">
            {items.length}
          </span>
        ) : null}
      </button>
      {open ? (
        <Popover
          anchorRef={anchorRef}
          className="menu-portal attention-popover"
          align="end"
          minWidth={340}
          onClose={close}
        >
          <div
            id={ATTENTION_PANEL_ID}
            ref={panelRef}
            role="dialog"
            aria-label="Attention"
            tabIndex={-1}
          >
            <section aria-labelledby="attention-items-cap">
              <h2 id="attention-items-cap" className="attention-cap">
                Needs attention
              </h2>
              {items.length ? (
                <ul className="attention-list">
                  {items.map((item) => (
                    <li key={item.id}>
                      <button
                        type="button"
                        className={`attention-row ${item.severity}`}
                        onClick={() => go(item.location)}
                      >
                        <Icon
                          name={item.severity === 'warning' ? 'alert' : 'info'}
                        />
                        <span className="t">
                          <b>{item.title}</b>
                          {item.detail ? <small>{item.detail}</small> : null}
                        </span>
                        <Icon name="chevronRight" className="go" />
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="attention-empty">Nothing needs attention.</p>
              )}
            </section>
            {showChecklist ? (
              <section aria-labelledby="attention-setup-cap">
                <div className="attention-cap-row">
                  <h2 id="attention-setup-cap" className="attention-cap">
                    Get set up
                  </h2>
                  <span className="attention-progress">
                    {done} of {counted.length} done
                  </span>
                  <button
                    type="button"
                    className="lnk"
                    title="Show it again from Settings › Preferences"
                    onClick={() => {
                      setChecklistHidden(true);
                      // The button goes with its section; the panel keeps
                      // the focus so Escape and Tab still work from it.
                      panelRef.current?.focus();
                    }}
                  >
                    Hide
                  </button>
                </div>
                <ul className="attention-list">
                  {checklist.map((step) => {
                    const body = (
                      <>
                        {step.done ? (
                          <Icon name="circleCheck" className="step-mark" />
                        ) : step.unavailable ? (
                          <Icon name="info" className="step-mark unavailable" />
                        ) : (
                          <span className="step-mark todo" aria-hidden="true" />
                        )}
                        <span className="t">
                          <b>
                            <span className="attention-state">
                              {step.done
                                ? 'Done: '
                                : step.unavailable
                                  ? 'Not available: '
                                  : 'To do: '}
                            </span>
                            {step.title}
                          </b>
                          <small>{step.detail}</small>
                        </span>
                      </>
                    );
                    return (
                      <li key={step.id}>
                        {!step.done && step.location ? (
                          <button
                            type="button"
                            className="attention-row step"
                            onClick={() => step.location && go(step.location)}
                          >
                            {body}
                            <Icon name="chevronRight" className="go" />
                          </button>
                        ) : (
                          <div
                            className={`attention-row step${step.done || step.unavailable ? ' done' : ''}`}
                          >
                            {body}
                          </div>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </section>
            ) : null}
            <section aria-labelledby="attention-recent-cap">
              <div className="attention-cap-row">
                <h2 id="attention-recent-cap" className="attention-cap">
                  Recent
                </h2>
                {recent.length && onClearRecent ? (
                  <button
                    type="button"
                    className="lnk"
                    onClick={() => {
                      onClearRecent();
                      panelRef.current?.focus();
                    }}
                  >
                    Clear
                  </button>
                ) : null}
              </div>
              {recent.length ? (
                <ol className="attention-recent">
                  {[...recent].reverse().map((record) => (
                    <li key={record.id} className={record.tone}>
                      {/* Colour alone does not mark a warning: the row has the
                          alert icon and a spoken prefix. */}
                      {record.tone === 'warning' ? (
                        <>
                          <Icon name="alert" className="tone" />
                          <span className="attention-state">Warning: </span>
                        </>
                      ) : null}
                      <span className="t">{record.message}</span>
                      <time dateTime={new Date(record.shownAt).toISOString()}>
                        {shownAt(record)}
                      </time>
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="attention-empty">
                  No messages in this session yet.
                </p>
              )}
              <p className="attention-note">
                Kept in memory until the app locks.
              </p>
            </section>
          </div>
        </Popover>
      ) : null}
    </>
  );
}
