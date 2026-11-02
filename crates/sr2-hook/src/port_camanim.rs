//! Ported functions (filled by a porting agent).

#![allow(unused_imports)]

use crate::portlib::*;
use crate::{detour, log};

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {}

/// Per-function call counts for the periodic log.
pub fn summary() -> String {
    String::new()
}
