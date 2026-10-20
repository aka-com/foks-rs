//! Explicit local platform qualification; no listener, secrets or state changes.
use foks_local_deadline::{sample, ClockSample, Deadline, DeadlineEnvelope};
use std::{
    io::Read,
    process::Command,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "brackets".into());
    match mode.as_str() {
        "sample" => println!("{}", serde_json::to_string(&sample()?)?),
        "envelope" => {
            let d = Deadline::capture(Instant::now() + Duration::from_secs(15))?;
            println!("{}", serde_json::to_string(d.envelope())?);
        }
        "receive" => {
            let mut text = String::new();
            std::io::stdin().take(2049).read_to_string(&mut text)?;
            if text.len() > 2048 {
                return Err("oversized envelope".into());
            }
            let envelope: DeadlineEnvelope = serde_json::from_str(&text)?;
            Deadline::receive(&envelope, Duration::from_secs(15))?.check()?;
            println!("accepted");
        }
        "brackets" => {
            let exe = std::env::current_exe()?;
            let mut maximum = 0;
            for _ in 0..100 {
                let before = sample()?;
                let output = Command::new(&exe).arg("sample").output()?;
                let after = sample()?;
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
                }
                if output.stdout.len() > 2048 {
                    return Err("oversized child sample".into());
                }
                let child: ClockSample = serde_json::from_slice(&output.stdout)?;
                if before.domain() != child.domain()
                    || child.domain() != after.domain()
                    || child.nanos() < before.nanos()
                    || child.nanos() > after.nanos()
                {
                    return Err("cross-process clock bracket/domain mismatch".into());
                }
                maximum = maximum.max(after.nanos() - before.nanos());
            }
            println!("100 cross-process brackets passed; maximum_ns={maximum}");
        }
        _ => return Err("expected sample, envelope, receive, or brackets".into()),
    }
    Ok(())
}
