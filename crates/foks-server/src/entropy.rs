pub trait Entropy: Send + Sync {
    fn fill(&self, destination: &mut [u8]) -> crate::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&self, destination: &mut [u8]) -> crate::Result<()> {
        getrandom::fill(destination)
            .map_err(|_| crate::Error::Config("operating-system entropy unavailable"))
    }
}
