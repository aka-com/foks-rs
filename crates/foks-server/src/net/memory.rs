//! Server policy adapters; request accounting has one shared implementation.
use foks_hosted_transport::memory::PoolLimits;
pub(crate) use foks_hosted_transport::memory::{
    current, retain_future, retain_work, Pools, Request, Workload,
};
use std::sync::Arc;

pub(crate) fn pools(limits: crate::SessionLimits) -> crate::Result<Arc<Pools>> {
    limits.validate()?;
    let checked = PoolLimits::new(
        limits.maximum_response_memory_bytes,
        limits.maximum_working_memory_bytes,
        limits.maximum_control_memory_bytes,
        limits.maximum_in_flight_requests,
    )
    .map_err(|_| crate::Error::Config("invalid session limit"))?;
    Ok(Pools::new(checked))
}

#[cfg(test)]
mod owned_list_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_geometry_preserves_server_defaults_and_stricter_validation() {
        let limits = crate::SessionLimits::default();
        let pools = pools(limits).unwrap();
        let available = foks_hosted_transport::memory::test_support::snapshot(&pools);
        assert_eq!(available.output, limits.maximum_response_memory_bytes);
        assert_eq!(
            available.working + available.small,
            limits.maximum_working_memory_bytes
        );
        assert_eq!(available.control, limits.maximum_control_memory_bytes);
        assert_eq!(available.bulk, 4);
        assert!(PoolLimits::new(1, 16 * 1024 * 1024, 1, 2).is_ok());
        let invalid = crate::SessionLimits {
            maximum_working_memory_bytes: 16 * 1024 * 1024,
            ..limits
        };
        assert!(matches!(
            super::pools(invalid),
            Err(crate::Error::Config("invalid session limit"))
        ));
    }
}
