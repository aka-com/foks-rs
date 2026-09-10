//! One source for local policy and durable hash/key namespaces.
pub use foks_client_db::ChatLimits;

// These values bind existing protected requests and consistency evidence.
// Renaming them is harmless; changing their values requires explicit handling
// of existing local operation material and retained anchors.
pub(super) const MESSAGE_ANCHOR_HASH_DOMAIN: u64 = 0x83a9_257e_a738_2011;
pub(super) const REQUEST_HASH_DOMAIN: u64 = 0xc34f_a132_2ff9_2311;
pub(super) const PROTECTED_MATERIAL_DOMAIN: &[u8] = b"chat-operation-v1";
