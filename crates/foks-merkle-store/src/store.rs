use crate::Result;

pub trait NodeReader {
    fn get_node(&self, hash: &[u8; 32]) -> Result<Option<Vec<u8>>>;
}

pub trait NodeWriter {
    fn put_node(&mut self, hash: [u8; 32], encoded: Vec<u8>) -> Result<()>;
}
