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

pub mod approval;
pub mod branch;
pub mod compare;
pub mod cooldown;
pub mod debounce;
pub mod for_each;
pub mod format;
pub mod http_request;
pub mod llm;
pub mod print;
pub mod variables;
pub mod wait;

/// Register the standard set into a product's registry.
pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(approval::spec());
    reg.register(branch::spec());
    reg.register(cooldown::spec());
    reg.register(debounce::spec());
    reg.register(compare::spec());
    reg.register(for_each::spec());
    reg.register(http_request::spec());
    reg.register(llm::ask_spec());
    reg.register(llm::decide_spec());
    reg.register(llm::extract_spec());
    reg.register(format::spec());
    reg.register(print::spec());
    reg.register(variables::get_spec());
    reg.register(variables::set_spec());
    reg.register(wait::spec());
}
