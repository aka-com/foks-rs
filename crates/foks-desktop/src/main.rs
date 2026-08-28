//! Toolkit-independent desktop backend shell.
//!
//! A future native/webview UI should invoke this same typed agent boundary;
//! this binary keeps UI toolkit dependencies out of the FOKS core crates.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use clap::Parser as _;
use foks_agent_client::AgentClient;
use foks_agent_proto::{Operation, ResponseResult};

#[derive(clap::Parser)]
#[command(name = "foks-desktop-backend")]
struct Arguments {
    #[arg(long)]
    agent_socket: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Status,
    Profiles,
    Accounts { profile: String },
    Sync { profile: String, alias: String },
    Kv { profile: String, alias: String },
    Teams { profile: String },
    TeamSync { profile: String, team_alias: String },
    RunJobs { profile: String },
}

fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("foks-desktop-backend: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    let operation = match arguments.command {
        Command::Status => Operation::Ping,
        Command::Profiles => Operation::ListProfiles,
        Command::Accounts { profile } => Operation::ListAccounts { profile },
        Command::Sync { profile, alias } => Operation::SyncAccount { profile, alias },
        Command::Kv { profile, alias } => Operation::ListKv { profile, alias },
        Command::Teams { profile } => Operation::ListTeams { profile },
        Command::TeamSync {
            profile,
            team_alias,
        } => Operation::SyncTeam {
            profile,
            team_alias,
        },
        Command::RunJobs { profile } => Operation::RunDueJobs { profile },
    };
    let response = AgentClient::new(arguments.agent_socket).call(operation)?;
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { code, message } => {
            Err(format!("agent returned {code:?}: {message}").into())
        }
    }
}
