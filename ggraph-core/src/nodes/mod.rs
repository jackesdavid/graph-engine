//! The standard node set — the nodes the engine ships with.
//!
//! These are part of the core, not a companion crate. An engine that cannot branch, loop,
//! compare or log is a topology library: the first thing anyone does with it is write those
//! four, and then there are two versions of `if` with different edge cases.
//!
//! Every node is one file, holding its port declaration, its implementation and its tests.
//! Adding one is adding a file and a line in [`register_all`].

use crate::host::Host;
use crate::registry::NodeRegistry;

pub mod branch;
pub mod compare;
pub mod for_each;
pub mod format;
pub mod print;

/// Register the standard set into a product's registry.
pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(branch::spec());
    reg.register(compare::spec());
    reg.register(for_each::spec());
    reg.register(format::spec());
    reg.register(print::spec());
}
