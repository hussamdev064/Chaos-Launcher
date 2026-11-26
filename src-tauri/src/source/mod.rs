//! Source checkout and user-asset management (Phase 3).
//!
//! Nothing here downloads game assets: the repository is upstream engine/game
//! source only, and `Data.rsdk` is always a file the user selected locally.

pub mod asset;
pub mod git;
