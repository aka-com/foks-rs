use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub busy_timeout: Duration,
    pub maximum_name_bytes: usize,
    pub maximum_blob_bytes: usize,
    pub maximum_receipt_bytes: usize,
    pub maximum_merkle_nodes_per_commit: usize,
    pub maximum_back_pointers: usize,
    pub maximum_kv_node_bytes: usize,
    pub maximum_kv_namespace_bytes: u64,
    pub maximum_kv_namespace_objects: u64,
    pub maximum_database_bytes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            busy_timeout: Duration::from_secs(5),
            maximum_name_bytes: 255,
            maximum_blob_bytes: 16 * 1024 * 1024,
            maximum_receipt_bytes: 16 * 1024 * 1024,
            maximum_merkle_nodes_per_commit: 131_072,
            maximum_back_pointers: 64,
            maximum_kv_node_bytes: 64 * 1024,
            maximum_kv_namespace_bytes: 16 * 1024 * 1024 * 1024,
            maximum_kv_namespace_objects: 1_000_000,
            maximum_database_bytes: 64 * 1024 * 1024 * 1024,
        }
    }
}
