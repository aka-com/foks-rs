//! A bounded in-memory log of the desktop backend's timings.
//!
//! Every agent operation the backend issues is recorded here with the
//! command that issued it, how long it took and how it ended. Nothing is
//! written to disk or sent anywhere: the renderer reads the log through the
//! `diagnostic_timings` command when the Refresh status popover is open, and
//! Copy diagnostics puts it on the clipboard beside the renderer's own log.
//!
//! What is recorded is what the popover already prints: operation and
//! command names, profiles (server ids) and outcome codes. Never request
//! fields, values, paths, aliases or error messages.

use foks_agent_proto::{ErrorCode, ResponseResult, ResponseTiming, TimerStatus};
use foks_desktop::AgentError as DesktopAgentError;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Events retained before the oldest is dropped.
pub const CAPACITY: usize = 4096;
const MAX_TEXT: usize = 64;
/// Background loops tracked for repeat reports. The agent has four; the cap
/// keeps a bad reply from growing the map without bound.
const MAX_TIMERS: usize = 16;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TimingValue {
    Number(f64),
    Bool(bool),
    Text(String),
}

/// One record, in the shape the renderer's log uses.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingEvent {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    pub layer: &'static str,
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<&'static str, TimingValue>,
}

/// The events after a cursor, and the cursor to continue from.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingBatch {
    pub events: Vec<TimingEvent>,
    pub next: u64,
}

#[derive(Default)]
struct Entries {
    entries: VecDeque<(u64, TimingEvent)>,
    next: u64,
}

#[derive(Default)]
pub struct TimingLog {
    entries: Mutex<Entries>,
    /// The start of the last pass recorded for each of the agent's loops, so
    /// a status read that reports the same pass again does not record it
    /// twice.
    timers: Mutex<BTreeMap<String, u64>>,
}

impl TimingLog {
    /// Records one event; the oldest is dropped once the log is full.
    pub fn record(&self, mut event: TimingEvent) {
        if let Some(scope) = &mut event.scope {
            scope.truncate(scope.floor_char_boundary(MAX_TEXT));
        }
        if let Some(code) = &mut event.code {
            code.truncate(code.floor_char_boundary(MAX_TEXT));
        }
        for value in event.attrs.values_mut() {
            if let TimingValue::Text(text) = value {
                text.truncate(text.floor_char_boundary(MAX_TEXT));
            }
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sequence = entries.next;
        entries.next += 1;
        entries.entries.push_back((sequence, event));
        while entries.entries.len() > CAPACITY {
            entries.entries.pop_front();
        }
    }

    /// The events recorded at or after `cursor`, oldest first.
    pub fn since(&self, cursor: u64) -> TimingBatch {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        TimingBatch {
            events: entries
                .entries
                .iter()
                .filter(|(sequence, _)| *sequence >= cursor)
                .map(|(_, event)| event.clone())
                .collect(),
            next: entries.next,
        }
    }

    /// Records newly reported background-loop executions. Agent status repeats
    /// the most recent execution for each loop, so the start timestamp is used
    /// for deduplication.
    pub fn record_agent_timers(&self, timers: &[TimerStatus]) {
        for timer in timers {
            let Some(started_at) = timer.started_at_ms.filter(|started| *started > 0) else {
                continue;
            };
            {
                let mut seen = self
                    .timers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if seen
                    .get(&timer.name)
                    .is_some_and(|last| *last >= started_at)
                {
                    continue;
                }
                if seen.len() >= MAX_TIMERS && !seen.contains_key(&timer.name) {
                    continue;
                }
                seen.insert(timer.name.clone(), started_at);
            }
            self.record(agent_timer(timer, started_at));
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .len()
    }
}

/// The command a transport was handed to, so every agent operation it
/// issues is recorded under that command and the profile it works on.
#[derive(Clone, Debug)]
pub struct Label {
    pub command: &'static str,
    pub scope: Option<String>,
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn millis(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 10_000.0).round() / 10.0
}

fn code_text(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{code:?}"))
}

/// The outcome and code an agent operation's result records.
pub fn outcome_of(
    result: Result<&ResponseResult, &DesktopAgentError>,
) -> (&'static str, Option<String>) {
    match result {
        Ok(ResponseResult::Success { .. }) => ("ok", None),
        Ok(ResponseResult::Error { code, .. }) => (
            if matches!(code, ErrorCode::Busy | ErrorCode::ProfileBusy) {
                "busy"
            } else {
                "error"
            },
            Some(code_text(*code)),
        ),
        Err(DesktopAgentError::Protocol { code, .. }) => (
            if matches!(code, ErrorCode::Busy | ErrorCode::ProfileBusy) {
                "busy"
            } else {
                "error"
            },
            Some(code_text(*code)),
        ),
        Err(DesktopAgentError::Cancelled) => ("cancelled", Some("cancelled".to_owned())),
        Err(DesktopAgentError::DeadlineExceeded) => ("error", Some("deadline-exceeded".to_owned())),
        Err(DesktopAgentError::Ambiguous(_)) => ("error", Some("ambiguous".to_owned())),
        Err(DesktopAgentError::Transport(_)) => ("error", Some("transport".to_owned())),
        Err(DesktopAgentError::Local(_)) => ("error", Some("local".to_owned())),
        Err(DesktopAgentError::Ipc { code, .. }) => ("error", Some(code.as_str().to_owned())),
    }
}

/// One agent operation as the backend saw it: the command it ran under, the
/// operation's name, its round trip, its outcome and, when the response
/// carried one, the agent's own phase timing.
pub fn agent_operation(
    label: Option<&Label>,
    operation: &'static str,
    elapsed: Duration,
    outcome: (&'static str, Option<String>),
    timing: Option<&ResponseTiming>,
) -> TimingEvent {
    let mut attrs = BTreeMap::new();
    attrs.insert("op", TimingValue::Text(operation.to_owned()));
    if let Some(label) = label {
        attrs.insert("command", TimingValue::Text(label.command.to_owned()));
    }
    if let Some(timing) = timing {
        let number = |value: u32| TimingValue::Number(f64::from(value));
        attrs.insert("queue", number(timing.queue_ms));
        attrs.insert("lock", number(timing.lock_ms));
        attrs.insert("body", number(timing.body_ms));
        if timing.pool_ms > 0 {
            attrs.insert("pool", number(timing.pool_ms));
        }
        if timing.start_ms > 0 {
            attrs.insert("start", number(timing.start_ms));
        }
        if timing.session_ms > 0 {
            attrs.insert("session", number(timing.session_ms));
        }
        if timing.lock_retries > 0 {
            attrs.insert("lock_retries", number(u32::from(timing.lock_retries)));
        }
        attrs.insert("auth", TimingValue::Bool(timing.auth_cached));
        attrs.insert("report", TimingValue::Bool(timing.report_cached));
        if let Some(behind) = &timing.waited_behind {
            attrs.insert("waited", TimingValue::Text(behind.clone()));
        }
        if timing.wait_ms > 0 || timing.prepare_ms > 0 {
            attrs.insert("prepare", number(timing.prepare_ms));
            attrs.insert("wait", number(timing.wait_ms));
            attrs.insert("rescope", number(timing.rescope_ms));
        }
        // The steps an operation measured inside itself go in one attribute
        // rather than one each: a record's attributes are few and short. A
        // step that took no measurable time states nothing, and one that
        // would not fit whole is left out rather than cut mid-number.
        let mut steps = String::new();
        for (name, elapsed) in timing.phases.iter().filter(|(_, elapsed)| *elapsed > 0) {
            let step = format!("{name}:{elapsed}");
            let separator = usize::from(!steps.is_empty());
            if steps.len() + separator + step.len() > MAX_TEXT {
                continue;
            }
            if separator > 0 {
                steps.push(',');
            }
            steps.push_str(&step);
        }
        if !steps.is_empty() {
            attrs.insert("phases", TimingValue::Text(steps));
        }
    }
    TimingEvent {
        at: now_millis(),
        layer: "backend",
        name: "agent.op",
        scope: label.and_then(|label| label.scope.clone()),
        phase: None,
        ms: Some(millis(elapsed)),
        outcome: Some(outcome.0),
        code: outcome.1,
        attrs,
    }
}

/// Converts one background-loop execution into a diagnostic timing event.
/// Background loops share profile admission with requests, and `due` records
/// the reported delay until the next tick.
pub fn agent_timer(timer: &TimerStatus, started_at: u64) -> TimingEvent {
    let mut attrs = BTreeMap::new();
    attrs.insert("runs", TimingValue::Number(timer.runs as f64));
    if timer.skips > 0 {
        attrs.insert("skips", TimingValue::Number(timer.skips as f64));
    }
    if let Some(due) = timer.next_due_in_ms {
        attrs.insert("due", TimingValue::Number(due as f64));
    }
    TimingEvent {
        at: started_at,
        layer: "agent",
        name: "agent.timer",
        scope: Some(timer.name.clone()),
        phase: None,
        ms: timer.duration_ms.map(f64::from),
        // Classify every non-successful loop outcome as an error.
        outcome: Some(match timer.outcome.as_deref() {
            Some("ok") => "ok",
            _ => "error",
        }),
        code: timer
            .outcome
            .as_deref()
            .filter(|outcome| *outcome != "ok")
            .map(str::to_owned),
        attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &'static str) -> TimingEvent {
        TimingEvent {
            at: 1,
            layer: "backend",
            name,
            scope: None,
            phase: None,
            ms: None,
            outcome: None,
            code: None,
            attrs: BTreeMap::new(),
        }
    }

    #[test]
    fn the_log_is_bounded_and_read_from_a_cursor() {
        let log = TimingLog::default();
        for _ in 0..(CAPACITY + 5) {
            log.record(event("a"));
        }
        assert_eq!(log.len(), CAPACITY);
        let all = log.since(0);
        assert_eq!(all.events.len(), CAPACITY);
        assert_eq!(all.next, (CAPACITY + 5) as u64);
        log.record(event("b"));
        let tail = log.since(all.next);
        assert_eq!(tail.events.len(), 1);
        assert_eq!(tail.events[0].name, "b");
        assert_eq!(log.since(tail.next).events.len(), 0);
    }

    #[test]
    fn text_fields_are_bounded() {
        let log = TimingLog::default();
        let mut long = event("a");
        long.scope = Some("s".repeat(200));
        long.attrs.insert("op", TimingValue::Text("o".repeat(200)));
        log.record(long);
        let recorded = &log.since(0).events[0];
        assert_eq!(recorded.scope.as_ref().unwrap().len(), MAX_TEXT);
        assert_eq!(
            match &recorded.attrs["op"] {
                TimingValue::Text(text) => text.len(),
                _ => 0,
            },
            MAX_TEXT
        );
    }

    fn timer(name: &str, started_at_ms: Option<u64>) -> TimerStatus {
        TimerStatus {
            name: name.to_owned(),
            started_at_ms,
            duration_ms: Some(240),
            outcome: Some("ok".to_owned()),
            next_due_in_ms: Some(28_000),
            runs: 3,
            skips: 1,
        }
    }

    #[test]
    fn each_reported_timer_pass_is_recorded_once_under_its_start() {
        let log = TimingLog::default();
        let reported = [
            timer("scheduler", Some(1_700_000_000_000)),
            timer("retention", None),
        ];
        log.record_agent_timers(&reported);
        // The pass that never ran contributes nothing.
        let events = log.since(0).events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "agent.timer");
        assert_eq!(events[0].layer, "agent");
        assert_eq!(events[0].at, 1_700_000_000_000);
        assert_eq!(events[0].scope.as_deref(), Some("scheduler"));
        assert_eq!(events[0].ms, Some(240.0));
        assert_eq!(events[0].outcome, Some("ok"));
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap()["attrs"],
            serde_json::json!({ "runs": 3.0, "skips": 1.0, "due": 28_000.0 })
        );
        // The same pass reported again is not recorded twice; a later one is.
        log.record_agent_timers(&reported);
        assert_eq!(log.since(0).events.len(), 1);
        log.record_agent_timers(&[timer("scheduler", Some(1_700_000_030_000))]);
        assert_eq!(log.since(0).events.len(), 2);
    }

    #[test]
    fn a_pass_the_agent_could_not_finish_is_recorded_as_an_error() {
        let log = TimingLog::default();
        let mut interrupted = timer("compatibility", Some(1_700_000_000_000));
        interrupted.outcome = Some("interrupted".to_owned());
        interrupted.skips = 0;
        log.record_agent_timers(&[interrupted]);
        let events = log.since(0).events;
        assert_eq!(events[0].outcome, Some("error"));
        assert_eq!(events[0].code.as_deref(), Some("interrupted"));
        assert!(!serde_json::to_string(&events[0]).unwrap().contains("skips"));
    }

    #[test]
    fn an_operation_records_its_command_outcome_and_code() {
        let label = Label {
            command: "list_profile_catalog",
            scope: Some("personal".to_owned()),
        };
        let ok = agent_operation(
            Some(&label),
            "ListKv",
            Duration::from_millis(12),
            outcome_of(Ok(&ResponseResult::Success {
                value: serde_json::Value::Null,
            })),
            Some(&ResponseTiming {
                queue_ms: 402,
                body_ms: 136,
                auth_cached: true,
                report_cached: true,
                waited_behind: Some("security-root".to_owned()),
                ..ResponseTiming::default()
            }),
        );
        assert_eq!(ok.scope.as_deref(), Some("personal"));
        assert_eq!(ok.ms, Some(12.0));
        assert_eq!(ok.outcome, Some("ok"));
        assert_eq!(ok.code, None);
        assert_eq!(
            serde_json::to_value(&ok).unwrap()["attrs"],
            serde_json::json!({
                "auth": true,
                "body": 136.0,
                "command": "list_profile_catalog",
                "lock": 0.0,
                "op": "ListKv",
                "queue": 402.0,
                "report": true,
                "waited": "security-root",
            })
        );
        let busy = agent_operation(
            None,
            "ListKv",
            Duration::from_millis(1),
            outcome_of(Err(&DesktopAgentError::Protocol {
                code: ErrorCode::ProfileBusy,
                message: "private detail".to_owned(),
                fields: Box::default(),
            })),
            None,
        );
        assert_eq!(busy.outcome, Some("busy"));
        assert_eq!(busy.code.as_deref(), Some("profile-busy"));
        assert!(!serde_json::to_string(&busy).unwrap().contains("private"));
    }

    #[test]
    fn an_operation_that_timed_itself_records_its_steps_as_one_attribute() {
        let label = Label {
            command: "reconcile_server",
            scope: Some("personal".to_owned()),
        };
        let event = agent_operation(
            Some(&label),
            "ReconcileProfile",
            Duration::from_millis(1_842),
            outcome_of(Ok(&ResponseResult::Success {
                value: serde_json::Value::Null,
            })),
            Some(&ResponseTiming {
                body_ms: 1_842,
                phases: vec![
                    ("admit".to_owned(), 4),
                    ("open".to_owned(), 131),
                    ("host".to_owned(), 1_702),
                    ("lease".to_owned(), 0),
                    ("fetch".to_owned(), 221),
                    ("apply".to_owned(), 37),
                ],
                ..ResponseTiming::default()
            }),
        );
        let steps = match &event.attrs["phases"] {
            TimingValue::Text(text) => text.clone(),
            other => panic!("the steps were not recorded as text: {other:?}"),
        };
        assert_eq!(steps, "admit:4,open:131,host:1702,fetch:221,apply:37");
        // One attribute, within the bounds a record is held to.
        assert!(steps.len() <= MAX_TEXT, "the steps do not fit: {steps}");
        assert_eq!(event.attrs.len(), 8);
        let log = TimingLog::default();
        log.record(event);
        assert_eq!(
            log.since(0).events[0].attrs["phases"],
            TimingValue::Text(steps)
        );
        // More steps than one attribute holds: the ones that fit are stated
        // whole, and none is cut mid-number.
        let many = agent_operation(
            None,
            "ReconcileProfile",
            Duration::from_millis(60_000),
            outcome_of(Ok(&ResponseResult::Success {
                value: serde_json::Value::Null,
            })),
            Some(&ResponseTiming {
                phases: (0..8)
                    .map(|index| (format!("step{index}"), 20_000 + index))
                    .collect(),
                ..ResponseTiming::default()
            }),
        );
        let TimingValue::Text(text) = &many.attrs["phases"] else {
            panic!("the steps were not recorded as text");
        };
        assert!(text.len() <= MAX_TEXT, "the steps do not fit: {text}");
        for step in text.split(',') {
            let (name, elapsed) = step.split_once(':').expect("a step without its time");
            assert!(name.starts_with("step"));
            assert_eq!(elapsed.parse::<u32>().unwrap() / 1_000, 20);
        }
        // An operation the request loop timed from the outside states no
        // steps of its own.
        let outside = agent_operation(
            None,
            "ListKv",
            Duration::from_millis(3),
            outcome_of(Ok(&ResponseResult::Success {
                value: serde_json::Value::Null,
            })),
            Some(&ResponseTiming::default()),
        );
        assert!(!outside.attrs.contains_key("phases"));
    }
}
