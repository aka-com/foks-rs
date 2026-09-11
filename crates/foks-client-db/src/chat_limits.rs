//! Fixed local chat admission policy shared by the journal and client workflows.
//! Wire and cryptographic limits remain in foks-proto.
pub struct ChatLimits;
impl ChatLimits {
    pub const CHANNELS: usize = 256;
    pub const PENDING_OPERATIONS: usize = 1000;
    pub const RETAINED_ANCHORS: usize = 10_000;
    pub const RECEIPT_BYTES: usize = 256;
    pub const HISTORY_ROWS: usize = 1000;
    pub const INBOX_ROWS: usize = 1000;
    pub const INBOX_PAGES: usize = 64;
    pub const HISTORY_BYTES: usize = 8 * 1024 * 1024;
    pub const PREDECESSORS: usize = 32;
    pub const RECOVERY_WINDOW: u64 = 100;
    pub const AUTHENTICATION_ATTEMPTS: usize = 3;
    pub const NAME_MIN_CHARS: usize = 3;
    pub const NAME_MAX_CHARS: usize = 32;
    pub const DESCRIPTION_MIN_CHARS: usize = 3;
    pub const DESCRIPTION_MAX_CHARS: usize = 512;
}

#[cfg(test)]
mod tests {
    use super::ChatLimits as L;
    #[test]
    fn local_limits_fit_wire_bounds_and_storage() {
        const {
            assert!(L::CHANNELS <= foks_proto::RT_MAX_COLLECTION);
            assert!(L::HISTORY_ROWS <= foks_proto::RT_MAX_COLLECTION);
            assert!(L::INBOX_ROWS <= foks_proto::RT_MAX_COLLECTION);
            assert!(L::PREDECESSORS <= foks_proto::RT_MAX_COLLECTION);
            assert!(L::RECOVERY_WINDOW <= L::HISTORY_ROWS as u64);
            assert!(L::HISTORY_BYTES + 65536 < foks_proto::RT_MAX_WIRE_BYTES);
            assert!(L::HISTORY_ROWS <= L::RETAINED_ANCHORS);
        }
        assert!(crate::schema::INITIAL.contains(&format!(
            "length(receipt) BETWEEN 1 AND {}",
            L::RECEIPT_BYTES
        )));
    }
}
