use crate::{Error, FailurePoint, Result};

pub(crate) fn inject(selected: Option<FailurePoint>, point: FailurePoint) -> Result<()> {
    if selected == Some(point) {
        Err(Error::Injected(point))
    } else {
        Ok(())
    }
}
