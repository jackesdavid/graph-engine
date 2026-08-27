// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Rounding: one number, or a column of them.
//!
//! Two nodes rather than one that looks at what it was handed. A node whose behaviour depends on
//! the shape of its input has a port that cannot say what it takes — and a port that cannot say
//! what it takes is a wire the editor cannot check.

mod each;
mod one;

use crate::host::Host;
use crate::registry::NodeRegistry;

pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(one::spec());
    reg.register(each::spec());
}
