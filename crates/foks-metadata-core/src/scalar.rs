//! Private scalar transport helpers; no public Value/SQL format changes.
/// Canonical i64 decimal text without an intermediate heap allocation.
pub(crate) struct Decimal {
    bytes: [u8; 20],
    start: usize,
}
impl Decimal {
    pub(crate) fn new(value: i64) -> Self {
        let mut result = Self {
            bytes: [0; 20],
            start: 20,
        };
        let mut magnitude = value.unsigned_abs();
        loop {
            result.start -= 1;
            result.bytes[result.start] = b'0' + (magnitude % 10) as u8;
            magnitude /= 10;
            if magnitude == 0 {
                break;
            }
        }
        if value < 0 {
            result.start -= 1;
            result.bytes[result.start] = b'-';
        }
        result
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes[self.start..]
    }
}

#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod codec;
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod response;
