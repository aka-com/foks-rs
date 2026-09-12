use foks_agent_client::AgentClient;
use foks_agent_proto::{sso::SsoAction, Operation, ResponseResult, SecretString};
use std::path::{Path, PathBuf};
#[derive(clap::Subcommand)]
pub enum SsoCommand {
    /// Enroll an identity-provider account directly on a security key.
    YubiSignup {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        card_serial: u32,
        #[arg(long, default_value_t = 0x82)]
        signing_slot: u8,
        #[arg(long, default_value_t = 0x83)]
        pq_slot: u8,
        #[arg(long)]
        pin_file: PathBuf,
        #[arg(long)]
        device_name: String,
        #[arg(long)]
        invite_file: Option<PathBuf>,
    },
    FinishYubiSignup {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: PathBuf,
    },
    /// Begin or resume browser authentication for an existing account.
    Login(Scope),
    /// Begin enrollment; the provider supplies the account username and email.
    Signup(Scope),
    /// Read the original flow without repeating a mutation.
    Status(Flow),
    /// Wait briefly for browser completion. Repeat while state is waiting.
    Poll(Flow),
    Cancel(Flow),
    /// Sign the completed browser flow and verify restored service access.
    FinishLogin {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        pin_file: Option<PathBuf>,
    },
    /// Commit the original enrollment, or reconcile its original signup journal.
    FinishSignup {
        #[command(flatten)]
        flow: Flow,
        #[arg(long)]
        device_name: String,
        #[arg(long)]
        invite_file: Option<PathBuf>,
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },
}
#[derive(clap::Args)]
pub struct Scope {
    #[arg(long)]
    profile: String,
    #[arg(long)]
    account: String,
}
#[derive(clap::Args)]
pub struct Flow {
    #[command(flatten)]
    scope: Scope,
    #[arg(long)]
    operation: String,
}
pub fn run(state: &Path, command: SsoCommand) -> Result<(), Box<dyn std::error::Error>> {
    let (scope, action) = match command {
        SsoCommand::YubiSignup {
            scope,
            card_serial,
            signing_slot,
            pq_slot,
            pin_file,
            device_name,
            invite_file,
        } => (
            scope,
            SsoAction::BeginYubiSignup {
                card_serial,
                signing_slot,
                pq_slot,
                pin: SecretString::new(super::read_pin(&pin_file)?.expose()),
                device_name,
                invite: SecretString::new(match invite_file {
                    Some(p) => super::read_passphrase(&p)?.to_string(),
                    None => String::new(),
                }),
            },
        ),
        SsoCommand::FinishYubiSignup { flow, pin_file } => (
            flow.scope,
            SsoAction::FinishYubiSignup {
                operation_id: flow.operation,
                pin: SecretString::new(super::read_pin(&pin_file)?.expose()),
            },
        ),
        SsoCommand::Login(scope) => (scope, SsoAction::Begin { for_login: true }),
        SsoCommand::Signup(scope) => (scope, SsoAction::Begin { for_login: false }),
        SsoCommand::Status(f) => (
            f.scope,
            SsoAction::Status {
                operation_id: f.operation,
            },
        ),
        SsoCommand::Poll(f) => (
            f.scope,
            SsoAction::Poll {
                operation_id: f.operation,
            },
        ),
        SsoCommand::Cancel(f) => (
            f.scope,
            SsoAction::Cancel {
                operation_id: f.operation,
            },
        ),
        SsoCommand::FinishLogin { flow, pin_file } => (
            flow.scope,
            SsoAction::FinishLogin {
                operation_id: flow.operation,
                pin: pin_file
                    .as_deref()
                    .map(super::read_pin)
                    .transpose()?
                    .map(|p| SecretString::new(p.expose().to_owned())),
            },
        ),
        SsoCommand::FinishSignup {
            flow,
            device_name,
            invite_file,
            passphrase_file,
        } => (
            flow.scope,
            SsoAction::FinishSignup {
                operation_id: flow.operation,
                device_name,
                invite: SecretString::new(match invite_file {
                    Some(p) => super::read_passphrase(&p)?.to_string(),
                    None => String::new(),
                }),
                passphrase: passphrase_file
                    .as_deref()
                    .map(super::read_passphrase)
                    .transpose()?
                    .map(|p| SecretString::new(p.to_string())),
            },
        ),
    };
    super::mcp::ensure_agent(state)?;
    let response = AgentClient::new(state.join("foks-rs.sock")).call(Operation::Sso {
        profile: scope.profile,
        account_alias: scope.account,
        action,
    })?;
    match response.result {
        ResponseResult::Success { value } => {
            println!("{}", serde_json::to_string_pretty(&value)?);
            Ok(())
        }
        ResponseResult::Error { message, .. } => Err(message.into()),
    }
}
