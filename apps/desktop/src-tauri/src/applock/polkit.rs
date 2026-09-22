//! Linux app-lock authentication via the PolicyKit (polkit) system D-Bus authority.

use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::Value;

use super::Capability;

const ACTION_ID: &str = "com.aka.foks.desktop.unlock";
const POLICY_FILE: &str = "com.aka.foks.desktop.policy";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";
const ALLOW_USER_INTERACTION: u32 = 1;

pub(super) fn capability() -> Capability {
    let unavailable = |reason| Capability {
        available: false,
        reason: Some(reason),
        mechanism: "none",
    };
    let connection = match Connection::system() {
        Ok(connection) => connection,
        Err(error) => {
            return unavailable(format!(
                "Polkit authentication requires the system bus, which is unavailable ({error})."
            ));
        }
    };
    match action_registered(&connection) {
        Ok(true) => Capability {
            available: true,
            reason: None,
            mechanism: "password",
        },
        Ok(false) => unavailable(
            "System authentication policy is not installed. Reinstall FOKS to enable application lock.".to_owned(),
        ),
        Err(error) => unavailable(format!("Polkit request failed: {error}")),
    }
}

fn action_registered(connection: &Connection) -> Result<bool, String> {
    type ActionDescription = (
        String,
        String,
        String,
        String,
        String,
        String,
        u32,
        u32,
        u32,
        HashMap<String, String>,
    );
    let reply = connection
        .call_method(
            Some(AUTHORITY),
            AUTHORITY_PATH,
            Some(AUTHORITY_INTERFACE),
            "EnumerateActions",
            &(""),
        )
        .map_err(|error| error.to_string())?;
    let actions: Vec<ActionDescription> = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    Ok(actions.iter().any(|action| action.0 == ACTION_ID))
}

fn subject() -> Result<(String, HashMap<String, Value<'static>>), String> {
    let pid = std::process::id();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|error| format!("Failed to read process start time: {error}"))?;
    let after_name = stat
        .rfind(')')
        .map(|at| &stat[at + 1..])
        .ok_or_else(|| "Unrecognized /proc stat format".to_owned())?;
    let start_time = after_name
        .split_whitespace()
        .nth(19)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| "Unrecognized /proc stat format".to_owned())?;
    Ok((
        "unix-process".to_owned(),
        HashMap::from([
            ("pid".to_owned(), Value::U32(pid)),
            ("start-time".to_owned(), Value::U64(start_time)),
        ]),
    ))
}

pub(super) fn authenticate(reason: &str) -> Result<bool, String> {
    let connection = Connection::system().map_err(|error| error.to_string())?;
    let details = HashMap::from([("polkit.message", reason)]);
    let reply = connection
        .call_method(
            Some(AUTHORITY),
            AUTHORITY_PATH,
            Some(AUTHORITY_INTERFACE),
            "CheckAuthorization",
            &(subject()?, ACTION_ID, details, ALLOW_USER_INTERACTION, ""),
        )
        .map_err(|error| error.to_string())?;
    let (authorized, _challenge, _details): (bool, bool, HashMap<String, String>) = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    Ok(authorized)
}
