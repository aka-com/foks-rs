//! Compile the same desktop chat policy consumed directly by TypeScript.
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=chat-limits.json");
    let source = fs::read_to_string("chat-limits.json").expect("read chat limits");
    let limits: std::collections::BTreeMap<String, usize> =
        serde_json::from_str(&source).expect("parse chat limits");
    let mut output = String::new();
    for (name, value) in limits {
        assert!(
            name.starts_with("CHAT_") && name.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        );
        output.push_str(&format!("pub const {name}: usize = {value};\n"));
    }
    let path = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    fs::write(path.join("chat_limits.rs"), output).expect("write chat limits");
}
