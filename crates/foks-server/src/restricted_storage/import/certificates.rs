//! Offline bounded proof that serial-key lookup equals exact-certificate lookup.
//! No TLS possession, stored/DER time equality, or current-owner linkage is inferred.
use super::{list::certificate, scan, unsupported, Result, ScanError};
use certificate::{CertificateError, CertificateWorkspace, MAX_DER};
use rusqlite::{params, Row};

#[derive(Clone, Copy)]
struct Serial {
    bytes: [u8; 20],
    length: u8,
}
impl Serial {
    fn from_row(row: &Row<'_>) -> Result<Self> {
        let value = row
            .get_ref(0)?
            .as_blob()
            .map_err(|_| unsupported("issued certificate serial type"))?;
        if value.is_empty() || value.len() > 20 {
            return Err(unsupported("issued certificate serial width"));
        }
        let mut bytes = [0; 20];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }
    fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
    fn magnitude(&self) -> &[u8] {
        let serial = self.bytes();
        let first = serial
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(serial.len() - 1);
        &serial[first..]
    }
}

fn parsed_error(error: CertificateError) -> ScanError {
    match error {
        CertificateError::Admission => ScanError::Allocation,
        CertificateError::Bounds
        | CertificateError::Malformed
        | CertificateError::Key
        | CertificateError::Serial
        | CertificateError::Expiry => unsupported("issued certificate restricted DER"),
    }
}

fn validate_next(
    db: &scan::Database,
    after: Option<&Serial>,
    workspace: &mut CertificateWorkspace,
) -> Result<Option<Serial>> {
    let control = db.control();
    control.statement()?;
    let sql = if after.is_some() {
        "SELECT serial,exact_certificate FROM main.issued_certificates WHERE serial>?1 ORDER BY serial LIMIT 1"
    } else {
        "SELECT serial,exact_certificate FROM main.issued_certificates ORDER BY serial LIMIT 1"
    };
    let mut statement = db.connection().prepare(sql)?;
    let mut rows = if let Some(after) = after {
        statement.query([after.bytes()])?
    } else {
        statement.query([])?
    };
    let Some(row) = rows.next()? else {
        control.check()?;
        return Ok(None);
    };
    let serial = Serial::from_row(row)?;
    let der = row
        .get_ref(1)?
        .as_blob()
        .map_err(|_| unsupported("issued certificate DER type"))?;
    if der.is_empty() || der.len() > MAX_DER {
        return Err(unsupported("issued certificate DER bytes"));
    }
    control.check()?;
    let parsed = workspace.parse(der).map_err(parsed_error)?;
    // The minimal magnitude is nonempty, including one zero byte for serial zero.
    // SQL permits any zero-padded width through twenty, not just the DER width.
    if serial.magnitude() != parsed.serial() {
        return Err(unsupported("issued certificate serial mismatch"));
    }
    control.check()?;
    // The X509 tree is already gone; this function drops its borrowed DER/cursor
    // before any next statement resets the shared VM work counter.
    Ok(Some(serial))
}

fn next_serial(db: &scan::Database, after: Option<&Serial>) -> Result<Option<Serial>> {
    let control = db.control();
    control.statement()?;
    let sql = if after.is_some() {
        "SELECT serial FROM main.issued_certificates WHERE serial>?1 ORDER BY serial LIMIT 1"
    } else {
        "SELECT serial FROM main.issued_certificates ORDER BY serial LIMIT 1"
    };
    let mut statement = db.connection().prepare(sql)?;
    let mut rows = if let Some(after) = after {
        statement.query([after.bytes()])?
    } else {
        statement.query([])?
    };
    let result = rows.next()?.map(Serial::from_row).transpose()?;
    control.check()?;
    Ok(result)
}

#[cfg(test)]
thread_local! {
    static AFTER_FIRST_PASS: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}
#[cfg(test)]
pub(super) fn after_first_pass(action: impl FnOnce() + 'static) {
    AFTER_FIRST_PASS.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(action));
    });
}

pub(super) fn audit(db: &scan::Database, workspace: &mut CertificateWorkspace) -> Result<()> {
    let control = db.control();
    control.check()?;
    let mut after = None;
    while let Some(serial) = validate_next(db, after.as_ref(), workspace)? {
        after = Some(serial);
    }
    #[cfg(test)]
    AFTER_FIRST_PASS.with(|slot| {
        if let Some(action) = slot.borrow_mut().take() {
            action();
        }
    });
    control.check()?;
    // The entire first pass is a prerequisite: now every exact DER has one
    // admitted magnitude, and every stored serial is its zero-padded form.
    after = None;
    while let Some(serial) = next_serial(db, after.as_ref())? {
        let magnitude = serial.magnitude();
        let mut padded = [0; 20];
        padded[20 - magnitude.len()..].copy_from_slice(magnitude);
        let mut matches = 0;
        for width in magnitude.len()..=20 {
            control.statement()?;
            // Both lookups use their primary key. Compare borrowed SQLite values,
            // without copying a DER or keeping an earlier cursor paused.
            let identical: bool = db.connection().query_row(
                "SELECT EXISTS(SELECT 1 FROM main.issued_certificates c WHERE c.serial=?1 AND c.exact_certificate=(SELECT s.exact_certificate FROM main.issued_certificates s WHERE s.serial=?2))",
                params![&padded[20 - width..], serial.bytes()],
                |row| row.get(0),
            )?;
            control.check()?;
            if identical {
                matches += 1;
                if matches > 1 {
                    return Err(unsupported("duplicate issued certificate DER"));
                }
            }
        }
        if matches != 1 {
            return Err(unsupported("issued certificate disappeared during audit"));
        }
        after = Some(serial);
    }
    control.check()
}
