//! Explicit native-state maintenance. Filesystem discovery never grants authority.
mod lease;
pub(crate) use lease::manifest_lock_file;
pub use lease::{ClientStateLease, ClientStateMaintenanceGuard};
pub(crate) mod files;
mod inventory;
pub(crate) mod trust;
pub use inventory::{inspect_native_state, ProfileInventoryReport, StateInventoryReport};

pub(crate) mod relocation;
pub use relocation::{
    recover_relocation, relocate_state, relocation_status, RelocationReport, RelocationStatus,
};

mod selection;
pub use selection::{default_desktop_state_root, selected_desktop_state_root};

mod archive_manifest;
mod export;
mod publication;
pub use export::{
    prepare_state_export, StateExportAuthorization, StateExportPreview, StateExportReport,
};
pub use foks_keystore::state_archive::StateTransferKey;

pub(crate) mod readiness;

pub(crate) mod import;

pub use import::{
    import_state, import_state_for_desktop, import_status, recover_import, StateImportReport,
    StateImportStatus,
};

mod verification;
pub use verification::{
    reauthenticate_imported_account, verify_imported_profile, ImportReauthenticationAction,
    ProfileVerificationReport, VerificationSecret,
};
mod secret_file;
pub use secret_file::{read_private_secret_file, read_transfer_key};
mod status;
pub use status::{maintenance_status, recover_state, MaintenanceStatus};
pub use verification::{imported_local_catalog, ImportedLocalCatalog};
