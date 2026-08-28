//! Real platform credential-service coverage.
//!
//! The test is ignored by default because it writes to the login Keychain or
//! desktop Secret Service. CI and developers opt in explicitly.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use foks_keystore::{Error, NativeCredentialStore};

const PRIMARY_KEY: &str = "integration.primary";
const BINARY_KEY: &str = "integration.binary";

#[test]
#[ignore = "requires an unlocked macOS Keychain or Linux Secret Service session"]
fn native_store_round_trips_replaces_isolates_and_cleans_up() {
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let primary_namespace = format!("integration-{suffix}");
    let isolated_namespace = format!("integration-isolated-{suffix}");
    let mut primary = NativeCredentialStore::open(&primary_namespace).unwrap();
    let mut reopened = NativeCredentialStore::open(&primary_namespace).unwrap();
    let mut isolated = NativeCredentialStore::open(&isolated_namespace).unwrap();

    // Cleanup is attempted whether the assertions below succeed or fail, so
    // CI does not accumulate credentials after an ordinary test failure.
    let outcome = exercise(&mut primary, &mut reopened, &mut isolated);
    let primary_cleanup = [PRIMARY_KEY, BINARY_KEY]
        .into_iter()
        .map(|key| primary.remove(key))
        .collect::<Vec<_>>();
    let isolated_cleanup = isolated.remove(PRIMARY_KEY);

    outcome.unwrap();
    assert!(primary_cleanup
        .into_iter()
        .all(|result| matches!(result, Ok(true))));
    assert!(matches!(isolated_cleanup, Ok(true)));
    assert!(matches!(primary.remove(PRIMARY_KEY), Ok(false)));
}

fn exercise(
    primary: &mut NativeCredentialStore,
    reopened: &mut NativeCredentialStore,
    isolated: &mut NativeCredentialStore,
) -> foks_keystore::Result<()> {
    let binary = [0, 1, 2, 0xff, 0, 0x80];
    step("put primary", primary.put(PRIMARY_KEY, b"first"))?;
    step("put binary", primary.put(BINARY_KEY, &binary))?;
    step("put isolated", isolated.put(PRIMARY_KEY, b"isolated"))?;
    require(step("get primary", reopened.get(PRIMARY_KEY))?.as_slice() == b"first")?;
    require(step("get binary", primary.get(BINARY_KEY))?.as_slice() == binary)?;
    require(step("get isolated", isolated.get(PRIMARY_KEY))?.as_slice() == b"isolated")?;

    reopened.put(PRIMARY_KEY, b"replacement")?;
    require(primary.get(PRIMARY_KEY)?.as_slice() == b"replacement")?;
    require(isolated.get(PRIMARY_KEY)?.as_slice() == b"isolated")?;

    require(primary.remove(PRIMARY_KEY)?)?;
    require(matches!(primary.get(PRIMARY_KEY), Err(Error::Missing)))?;
    primary.put(PRIMARY_KEY, b"replacement")?;
    Ok(())
}

fn step<T>(label: &str, result: foks_keystore::Result<T>) -> foks_keystore::Result<T> {
    result.map_err(|error| Error::Native(format!("{label}: {error}")))
}

fn require(condition: bool) -> foks_keystore::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Native(
            "native credential integration assertion failed".to_owned(),
        ))
    }
}
