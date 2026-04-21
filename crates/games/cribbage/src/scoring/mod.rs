//! Pure Cribbage show-phase scoring, one rule per file.

pub mod fifteens;
pub mod flush;
pub mod nobs;
pub mod pairs;
pub mod runs;
pub mod show;

pub use show::{ShowScore, score_hand};
