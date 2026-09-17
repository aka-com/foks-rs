use std::{
    path::Path,
    process::{Command, Output},
    time::{Duration, Instant},
};
pub fn cli(state: &Path, args: &[&str]) -> Output {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let output = Command::new(env!("CARGO_BIN_EXE_foks-rs"))
            .arg("--json")
            .arg("--state-dir")
            .arg(state)
            .args(args)
            .output()
            .unwrap();
        // Busy is a checked-session non-admission result. Commands reserving a
        // one-time output file must not be repeated after filesystem side effects.
        if args.contains(&"--output")
            || !String::from_utf8_lossy(&output.stderr)
                .contains("Another operation is using this profile.")
            || Instant::now() >= deadline
        {
            return output;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
