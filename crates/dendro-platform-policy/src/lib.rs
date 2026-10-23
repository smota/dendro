//! Pure platform admission policy shared by safe core and isolated native I/O.
#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePlatform {
    Windows,
    Linux,
    Macos,
}

mod policy;
pub use policy::*;
