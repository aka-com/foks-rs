//! Desktop alerts through the freedesktop.org notification service
//! (`org.freedesktop.Notifications` on the session bus), which GNOME Shell,
//! KDE Plasma, dunst, mako and other notification daemons implement.
//!
//! A channel's alerts share one entry: each alert replaces the entry the
//! channel already shows (`replaces_id`), and its text states every message
//! the entry accounts for. The entry is forgotten when the service reports
//! it closed, so the channel's next alert starts a new one.
//!
//! D-Bus calls run on one worker thread, in the order `display` and `clear`
//! queue them, and never while `LocalState` is held: the worker and the
//! signal listener take `SHARED` only between calls, and never call into
//! `chat_local` while holding it.
use super::super::{activate, delivery_failed, error, grouped_text, route_current, Alert};
use crate::agent::AgentError;
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

/// The name the backend reports to the session state.
pub const BACKEND: &str = "freedesktop";
const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
/// The packaged desktop entry, `foks-desktop.desktop`, without its suffix;
/// the service takes the alert's icon and application name from it.
const DESKTOP_ENTRY: &str = "foks-desktop";
/// The action a click on the entry invokes. Services that draw buttons
/// label it Open.
const DEFAULT_ACTION: &str = "default";
/// How long any one call to the bus or the service may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// The most characters of the service's own name the session reports.
const SERVICE_NAME_CHARS: usize = 64;

#[derive(Default)]
struct Shared {
    connection: Option<Connection>,
    /// The service the last probe reached: its name, and whether it reads
    /// markup in the body.
    service: Option<Service>,
    /// The bus's unique name for the service, which the signals it emits
    /// carry as their sender.
    owner: Option<String>,
    shown: Shown,
}
#[derive(Clone)]
struct Service {
    name: Option<String>,
    markup: bool,
}
enum Job {
    Show(Alert),
    Clear,
}
static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();

fn shared() -> MutexGuard<'static, Shared> {
    SHARED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether the last probe reached a notification service.
pub fn available() -> bool {
    shared().service.is_some()
}
/// The name the notification service gives itself, such as "GNOME Shell".
pub fn service() -> Option<String> {
    shared().service.as_ref().and_then(|s| s.name.clone())
}
/// Probes the service again, so a daemon started after FOKS is found.
pub async fn refresh() {
    let _ = tauri::async_runtime::spawn_blocking(probe).await;
}
pub fn install(app: &tauri::AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    let _ = QUEUE.set(Mutex::new(tx));
    let worker = app.clone();
    let _ = std::thread::Builder::new()
        .name("foks-notifications".into())
        .spawn(move || {
            for job in rx {
                match job {
                    Job::Show(alert) => show(&worker, alert),
                    Job::Clear => close_all(),
                }
            }
        });
}
/// The service has no permission of its own: alerts are permitted while one
/// is running.
pub async fn permission() -> bool {
    refresh().await;
    available()
}
pub fn refusal() -> AgentError {
    error("No desktop notification service was found. Start one, then enable desktop alerts again.")
}
pub fn display(_: &tauri::AppHandle, alert: &Alert) -> Result<(), AgentError> {
    if !available() {
        return Err(error("No desktop notification service was found."));
    }
    enqueue(Job::Show(alert.clone()))
}
pub fn clear() {
    let _ = enqueue(Job::Clear);
}
pub fn open_settings() -> Result<(), AgentError> {
    Err(error(
        "Notification settings are unavailable on this platform.",
    ))
}

fn enqueue(job: Job) -> Result<(), AgentError> {
    let queue = QUEUE
        .get()
        .ok_or_else(|| error("Desktop alerts are not started."))?;
    queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send(job)
        .map_err(|_| error("Desktop alerts stopped."))
}

/// The session bus connection, made on the first probe and kept until the
/// bus closes it; the signal listener starts with it.
fn connection() -> Option<Connection> {
    if let Some(connection) = shared().connection.clone() {
        return Some(connection);
    }
    let connection = zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .ok()?;
    {
        let mut shared = shared();
        if let Some(existing) = shared.connection.clone() {
            return Some(existing);
        }
        shared.connection = Some(connection.clone());
    }
    if let Some(app) = APP.get() {
        let (listener, app) = (connection.clone(), app.clone());
        let _ = std::thread::Builder::new()
            .name("foks-notification-signals".into())
            .spawn(move || listen(&app, &listener));
    }
    Some(connection)
}

fn probe() {
    let found = connection().and_then(|connection| {
        let service = server(&connection).ok()?;
        Some((service, name_owner(&connection)))
    });
    let mut shared = shared();
    match found {
        Some((service, owner)) => {
            shared.service = Some(service);
            shared.owner = owner;
        }
        None => {
            shared.service = None;
            shared.owner = None;
        }
    }
}

fn server(connection: &Connection) -> zbus::Result<Service> {
    let (name, _vendor, _version, _spec): (String, String, String, String) = connection
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "GetServerInformation",
            &(),
        )?
        .body()
        .deserialize()?;
    let capabilities: Vec<String> = connection
        .call_method(Some(SERVICE), PATH, Some(INTERFACE), "GetCapabilities", &())?
        .body()
        .deserialize()?;
    Ok(Service {
        name: service_name(&name),
        markup: capabilities.iter().any(|c| c == "body-markup"),
    })
}

fn name_owner(connection: &Connection) -> Option<String> {
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(SERVICE),
        )
        .ok()?
        .body()
        .deserialize::<String>()
        .ok()
}

/// The service's self-reported name, without control characters and bounded,
/// or `None` when nothing printable is left.
fn service_name(raw: &str) -> Option<String> {
    let name: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(SERVICE_NAME_CHARS)
        .collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Escapes the characters a service that reads body markup would interpret,
/// so message text is shown as written.
fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn show(app: &tauri::AppHandle, alert: Alert) {
    // A route that a lock, a settings change or a newer session has retired
    // since the alert was queued is not shown.
    if !route_current(app, &alert.token) {
        return;
    }
    let (connection, service, entry) = {
        let shared = shared();
        (
            shared.connection.clone(),
            shared.service.clone(),
            shared.shown.prepare(&alert),
        )
    };
    let (Some(connection), Some(service)) = (connection, service) else {
        delivery_failed(app);
        return;
    };
    let body = if service.markup {
        escape_markup(&entry.body)
    } else {
        entry.body.clone()
    };
    let reply = notify(&connection, entry.replaces, &alert.title, &body);
    let Ok(id) = reply else {
        delivery_failed(app);
        return;
    };
    shared().shown.record(id, &alert, &entry);
    if !route_current(app, &alert.token) {
        close(&connection, id);
        shared().shown.closed(id);
    }
}

/// Shows an entry, or replaces the one `replaces` names (0 for none), and
/// returns its id.
fn notify(connection: &Connection, replaces: u32, title: &str, body: &str) -> zbus::Result<u32> {
    let hints = HashMap::from([
        ("category", Value::from("im.received")),
        ("desktop-entry", Value::from(DESKTOP_ENTRY)),
    ]);
    connection
        .call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "Notify",
            &(
                "FOKS",
                replaces,
                "",
                title,
                body,
                vec![DEFAULT_ACTION, "Open"],
                hints,
                -1i32,
            ),
        )?
        .body()
        .deserialize::<u32>()
}

fn close(connection: &Connection, id: u32) {
    let _ = connection.call_method(
        Some(SERVICE),
        PATH,
        Some(INTERFACE),
        "CloseNotification",
        &(id),
    );
}

fn close_all() {
    let (connection, ids) = {
        let mut shared = shared();
        (shared.connection.clone(), shared.shown.take())
    };
    if let Some(connection) = connection {
        for id in ids {
            close(&connection, id);
        }
    }
}

/// Follows the service's ActionInvoked and NotificationClosed signals for
/// the entries this process shows. Signals from any other bus name are
/// ignored, so another client cannot activate a route.
fn listen(app: &tauri::AppHandle, connection: &Connection) {
    let Ok(rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(INTERFACE)
        .and_then(|rule| rule.path(PATH))
        .map(|rule| rule.build())
    else {
        return;
    };
    let Ok(signals) = zbus::blocking::MessageIterator::for_match_rule(rule, connection, None)
    else {
        return;
    };
    for message in signals {
        let Ok(message) = message else {
            break;
        };
        let header = message.header();
        let sender = header.sender().map(|s| s.to_string());
        if sender.is_none() || !from_service(connection, sender.as_deref()) {
            continue;
        }
        match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => {
                let Ok((id, _action)) = message.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                let token = shared().shown.closed(id);
                if let Some(token) = token {
                    activate(app, &token);
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _reason)) = message.body().deserialize::<(u32, u32)>() {
                    shared().shown.closed(id);
                }
            }
            _ => {}
        }
    }
    // The bus connection ended: the next probe makes a new one.
    let mut shared = shared();
    shared.connection = None;
    shared.service = None;
    shared.owner = None;
}

/// Whether a signal's sender is the service's current owner, asking the bus
/// again when it is not the owner last seen (the service restarted).
fn from_service(connection: &Connection, sender: Option<&str>) -> bool {
    if shared().owner.as_deref() == sender {
        return true;
    }
    let owner = name_owner(connection);
    let matches = owner.as_deref() == sender;
    shared().owner = owner;
    matches
}

/// The entries this process shows, by notification id and by channel.
#[derive(Default)]
struct Shown {
    /// By notification id: the route token the entry activates.
    ids: HashMap<u32, String>,
    /// By channel key: the entry that shows the channel's alerts.
    groups: HashMap<String, Group>,
}
struct Group {
    id: u32,
    count: u32,
    incomplete: bool,
}
/// What an alert is shown as: the entry it replaces (0 for a new one) and
/// the channel's totals with this alert counted.
#[derive(Debug, PartialEq)]
struct Entry {
    replaces: u32,
    count: u32,
    incomplete: bool,
    body: String,
}
impl Shown {
    fn prepare(&self, alert: &Alert) -> Entry {
        let (replaces, count, incomplete) = match self.groups.get(&alert.group) {
            Some(group) => (
                group.id,
                group.count.saturating_add(alert.count),
                group.incomplete || alert.incomplete,
            ),
            None => (0, alert.count, alert.incomplete),
        };
        Entry {
            replaces,
            count,
            incomplete,
            body: grouped_text(alert.preview.as_deref(), count, incomplete),
        }
    }
    fn record(&mut self, id: u32, alert: &Alert, entry: &Entry) {
        if entry.replaces != 0 && entry.replaces != id {
            self.ids.remove(&entry.replaces);
        }
        self.ids.insert(id, alert.token.clone());
        self.groups.insert(
            alert.group.clone(),
            Group {
                id,
                count: entry.count,
                incomplete: entry.incomplete,
            },
        );
    }
    /// Forgets a closed entry, returning the route token it activated.
    fn closed(&mut self, id: u32) -> Option<String> {
        self.groups.retain(|_, group| group.id != id);
        self.ids.remove(&id)
    }
    /// Forgets every entry, returning their ids to close.
    fn take(&mut self) -> Vec<u32> {
        self.groups.clear();
        self.ids.drain().map(|(id, _)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(token: &str, group: &str, preview: Option<&str>, count: u32) -> Alert {
        Alert {
            token: token.into(),
            group: group.into(),
            title: "FOKS".into(),
            preview: preview.map(Into::into),
            count,
            incomplete: false,
        }
    }

    #[test]
    fn a_channel_keeps_one_entry_that_counts_every_message() {
        let mut shown = Shown::default();
        let first = alert("t1", "general", None, 1);
        let entry = shown.prepare(&first);
        assert_eq!(entry.replaces, 0);
        assert_eq!(entry.body, "New chat message.");
        shown.record(7, &first, &entry);
        // The second and third alerts replace the first entry in place.
        let second = alert("t2", "general", None, 1);
        let entry = shown.prepare(&second);
        assert_eq!((entry.replaces, entry.count), (7, 2));
        assert_eq!(entry.body, "2 new chat messages.");
        shown.record(7, &second, &entry);
        let more = alert("t3", "general", None, 4);
        let entry = shown.prepare(&more);
        assert_eq!(entry.body, "6 new chat messages.");
        shown.record(9, &more, &entry);
        // A service that issued a new id retires the old one.
        assert_eq!(shown.ids.len(), 1);
        assert_eq!(shown.ids.get(&9).map(String::as_str), Some("t3"));
        // Another channel has its own entry.
        let other = shown.prepare(&alert("t4", "random", None, 1));
        assert_eq!(other.replaces, 0);
        // Closed, the channel's next alert starts a new entry.
        assert_eq!(shown.closed(9).as_deref(), Some("t3"));
        assert_eq!(shown.prepare(&alert("t5", "general", None, 1)).replaces, 0);
    }

    #[test]
    fn previews_show_the_newest_message_and_incomplete_counts_say_so() {
        let mut shown = Shown::default();
        let first = alert("t1", "general", Some("priya.n: hello"), 1);
        let entry = shown.prepare(&first);
        assert_eq!(entry.body, "priya.n: hello");
        shown.record(3, &first, &entry);
        let summary = Alert {
            incomplete: true,
            ..alert("t2", "general", None, 2)
        };
        let entry = shown.prepare(&summary);
        assert_eq!(
            entry.body,
            "3 new chat messages. Some messages could not be checked."
        );
        shown.record(3, &summary, &entry);
        assert_eq!(shown.take(), vec![3]);
        assert!(shown.groups.is_empty());
    }

    #[test]
    fn service_text_is_escaped_and_bounded() {
        assert_eq!(
            escape_markup("<b>a</b> & \"q\" 'x'"),
            "&lt;b&gt;a&lt;/b&gt; &amp; &quot;q&quot; &apos;x&apos;"
        );
        assert_eq!(
            service_name(" GNOME\u{7} Shell "),
            Some("GNOME Shell".into())
        );
        assert_eq!(service_name("\n\t"), None);
        assert_eq!(
            service_name(&"x".repeat(100)).map(|n| n.chars().count()),
            Some(SERVICE_NAME_CHARS)
        );
    }
}
