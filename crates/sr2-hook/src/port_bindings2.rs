//! Port module: filled in by a porting subagent. See portlib.rs for the shared helpers.

#![allow(dead_code, unused_imports)]

use crate::portlib::*;
use crate::{detour, log};

/// Install this module's replacements (called from hooks::install_all).
pub fn install() {}

/// One line for the periodic log.
pub fn summary() -> String {
    String::new()
}
