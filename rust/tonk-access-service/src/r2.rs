//! R2 operations module.

pub mod presign;

pub use presign::{Method, R2Config};
pub use s3_presign::Checksum;
