//! Monotonic accounting shared by one host operation across SQL intervals.
//! This value neither executes SQL nor establishes connection or transaction
//! ownership. The enclosing host must retain the same value until completion.

use crate::{Error, Result, MAX_SQL_CALLS};

/// Counts attempted host calls and prepaid cleanup without a reset or refund.
///
/// A cleanup reservation can authorize only the host's one explicit ROLLBACK;
/// it says nothing about SQLite's implicit rollback or connection destruction.
#[repr(C)]
pub struct SqlCallBudget {
    charged: u16,
    attempts: u16,
    cleanup_credit: u8,
    failed: bool,
}

const _: () = {
    assert!(MAX_SQL_CALLS <= u16::MAX as usize);
    assert!(std::mem::size_of::<SqlCallBudget>() == 6);
    assert!(std::mem::align_of::<SqlCallBudget>() == 2);
};

impl Default for SqlCallBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl SqlCallBudget {
    pub const fn new() -> Self {
        Self {
            charged: 0,
            attempts: 0,
            cleanup_credit: 0,
            failed: false,
        }
    }

    pub fn charged(&self) -> usize {
        usize::from(self.charged)
    }

    /// Includes attempts that fail parameter validation, preparation or execution.
    /// Unused prepaid cleanup and refused calls are not attempted SQL calls.
    pub fn attempts(&self) -> usize {
        usize::from(self.attempts)
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn fail(&mut self) {
        self.failed = true;
    }

    fn refuse<T>(&mut self) -> Result<T> {
        self.fail();
        Err(Error::Bounds)
    }

    fn charge(&mut self) -> Result<()> {
        if self.failed || self.charged() >= MAX_SQL_CALLS {
            return self.refuse();
        }
        self.charged = match self.charged.checked_add(1) {
            Some(next) => next,
            None => return self.refuse(),
        };
        Ok(())
    }

    fn count_attempt(&mut self) -> Result<()> {
        self.attempts = match self.attempts.checked_add(1) {
            Some(next) if next <= self.charged => next,
            _ => return self.refuse(),
        };
        Ok(())
    }

    /// Call before validation, binding allocation or preparing an ordinary call.
    pub fn attempt(&mut self) -> Result<()> {
        self.charge()?;
        self.count_attempt()
    }

    /// Prepay one cleanup attempt before BEGIN. Nested reservations are refused.
    pub fn begin_cleanup_reservation(&mut self) -> Result<()> {
        if self.cleanup_credit != 0 {
            return self.refuse();
        }
        self.charge()?;
        self.cleanup_credit = 1;
        Ok(())
    }

    /// Consume the reserved credit before the sole explicit ROLLBACK attempt.
    /// This remains possible after ordinary admission fails. Failure to roll
    /// back must invalidate the physical owner; the credit cannot be retried.
    pub fn attempt_rollback(&mut self) -> Result<()> {
        if self.cleanup_credit != 1 {
            return self.refuse();
        }
        self.cleanup_credit = 0;
        self.count_attempt()
    }

    /// Disarm unused cleanup after a successful COMMIT or a failed BEGIN.
    /// The original charge remains spent and failure remains sticky.
    pub fn finish_cleanup_reservation(&mut self) -> Result<()> {
        if self.cleanup_credit != 1 {
            return self.refuse();
        }
        self.cleanup_credit = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
