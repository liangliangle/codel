//! Vendor compatibility configuration; `codel-config` owns it.

pub(crate) use codel_config::compat::INSTRUCTION_FILENAMES;
pub use codel_config::compat::{
    COMPAT_CELLS, CompatCell, CompatConfig, CompatConfigToml, CompatRemoteKey, CompatSurface,
    CompatVendor, VendorCompat, VendorCompatToml,
};
