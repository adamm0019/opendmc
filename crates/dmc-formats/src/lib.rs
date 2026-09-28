//! Clean-room readers for the data formats of *Devil May Cry* (2001) as shipped
//! in the *Devil May Cry HD Collection*.
//!
//! Every reader works on a byte slice supplied by the caller, checks every
//! access against its bounds, and takes the file's byte order at runtime:
//! the PS3 build is big-endian and the PC build little-endian, with some
//! records widened for 64-bit (see `docs/formats/README.md`).
//!
//! Each format module also has a writer. The writers produce the synthetic
//! fixtures used in tests (no game data is ever checked in), and they are the
//! starting point for mod tooling.

pub mod bdp;
pub mod bytes;
pub mod camera;
pub mod collision;
pub mod dds;
pub mod detect;
pub mod dxt;
pub mod error;
pub mod geometry;
pub mod ik;
pub mod model;
pub mod motion;
pub mod props;
pub mod room;
pub mod texture;

pub use bytes::{Endian, Reader, Writer};
pub use error::{FormatError, Result};
