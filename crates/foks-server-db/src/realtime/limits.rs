//! Fixed service admission policy. Wire/cryptographic limits belong to foks-proto.
//! Changing these requires reviewing the schema and bounded response encoding.
pub struct RealtimeLimits;
impl RealtimeLimits {
    pub const CHANNELS_PER_TEAM: usize = 256;
    /// Bounds scanned eligible membership rows, including non-user parties.
    pub const FANOUT_MEMBERS: usize = 1024;
    pub const CHANNEL_CONFIGURATION_BYTES: usize = 16 * 1024;
    pub const CHANNEL_ACTIVITY_BYTES: usize = 256;
    pub const STORED_MESSAGE_BYTES: usize = foks_proto::RT_MAX_REQUEST_BYTES;
    pub const DEFAULT_RECENTS_ROWS: usize = 100;
    pub const DEFAULT_INBOX_ROWS: usize = 100;
    pub const INBOX_ROWS: usize = 1000;
    pub const INBOX_SCAN_ROWS: usize = 4096;
    pub const INBOX_RECONCILE_CHANNELS: usize = 4096;
    pub const INBOX_MEMBERSHIP_BYTES: usize = 1024 * 1024;
    pub const HISTORY_ROWS: usize = 1000;
    /// Sum of stored message encodings, excluding the small response container.
    pub const HISTORY_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
}

#[cfg(test)]
mod tests {
    use super::RealtimeLimits as L;
    use foks_proto::*;

    #[test]
    fn service_limits_fit_wire_containers_and_schema() {
        // Include conservative framing costs for each possible collection.
        const {
            assert!(L::DEFAULT_RECENTS_ROWS <= L::HISTORY_ROWS);
            assert!(L::DEFAULT_INBOX_ROWS <= L::INBOX_ROWS);
            assert!(L::INBOX_ROWS <= L::INBOX_SCAN_ROWS);
            assert!(L::INBOX_ROWS <= RT_MAX_COLLECTION);
            assert!(L::HISTORY_ROWS <= RT_MAX_COLLECTION);
            assert!(L::CHANNELS_PER_TEAM <= RT_MAX_COLLECTION);
            assert!(L::HISTORY_MESSAGE_BYTES + (RT_MAX_COLLECTION + 1) * 16 < RT_MAX_WIRE_BYTES);
            assert!(
                L::CHANNELS_PER_TEAM
                    * (L::CHANNEL_CONFIGURATION_BYTES + L::CHANNEL_ACTIVITY_BYTES + 16)
                    + 128
                    < RT_MAX_WIRE_BYTES
            );
            assert!(RT_MAX_CIPHERTEXT_BYTES + 512 <= L::STORED_MESSAGE_BYTES);
        }
        let sql = include_str!("../schema/realtime.sql");
        for (column, limit) in [
            ("metadata", L::CHANNEL_CONFIGURATION_BYTES),
            ("last_message", L::CHANNEL_ACTIVITY_BYTES),
            ("envelope", L::STORED_MESSAGE_BYTES),
            ("exact_message", L::STORED_MESSAGE_BYTES),
        ] {
            assert!(
                sql.contains(&format!("length({column}) <= {limit}")),
                "schema bound differs for {column}"
            );
        }
        let last = RtLastMessage {
            sequence: i64::MAX as u64,
            kind: RtMessageType::Basic,
            insert_time: i64::MAX as u64,
            sender: Some(
                RtPartyId::new(
                    EntityId::from_bytes({
                        let mut id = vec![255; 33];
                        id[0] = ENTITY_USER;
                        id
                    })
                    .unwrap(),
                )
                .unwrap(),
            ),
            further_user_attribution: None,
        };
        assert!(last.encoded().unwrap().len() <= L::CHANNEL_ACTIVITY_BYTES);
    }
}
