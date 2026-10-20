//! Owned List allocation interface. These functions require a current credential
//! and caller-owned SQL scope, but do not establish schema, TLS, fence, durability
//! or publication authority. Supply the original admitted request owner; no
//! returned bytes may be published merely because allocation succeeded.
//!
//! Both helper and framer install that owner's allocation hooks synchronously.
//! The server still owns its execution/deadline scope and final physical output.

pub use crate::list_encoder::owned::{
    frame_owned_list, ListRetainedOwner, OwnedFrameBytes, OwnedListBytes,
};
pub use crate::personal::read_personal_list_owned;
