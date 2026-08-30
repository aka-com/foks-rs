//! Strict server-side decoders for the supported v0.1.9 request surface.

mod compat;
mod federation;
mod generic;
mod identity;
mod invites;
mod merkle;
mod passphrase;
mod recovery;
mod team_admin;
mod team_loader;
mod user_mutation;
mod yubi;

pub use compat::*;
pub use federation::*;
pub use generic::*;
pub use identity::*;
pub use invites::*;
pub use merkle::*;
pub use passphrase::*;
pub use recovery::*;
pub use team_admin::*;
pub use team_loader::*;
pub use user_mutation::*;
pub use yubi::*;
