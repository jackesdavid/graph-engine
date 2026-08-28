// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

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

pub mod services;

pub mod approval;
pub mod branch;
pub mod compare;
pub mod cooldown;
pub mod debounce;
pub mod for_each;
pub mod format;
pub mod http_request;
pub mod llm;
pub mod pick;
pub mod print;
pub mod report;
pub mod round;
pub mod schema;
mod table;
pub mod variables;
pub mod wait;

/// Register the standard set into a product's registry.
///
/// `services` is what these nodes need from the world — an approval channel, a network, a model,
/// a table store. They are a parameter rather than something reached through [`Host`] because
/// the scheduler never touches them, and a product that wants the table nodes but not the model
/// ones should be able to say so. [`Services::none`](services::Services::none) supplies none of
/// them, and every node that needs one then refuses with a message naming what is missing.
pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>, services: &services::Services) {
    reg.register(approval::spec(services));
    reg.register(branch::spec(services));
    reg.register(cooldown::spec(services));
    reg.register(debounce::spec(services));
    reg.register(compare::spec(services));
    reg.register(for_each::spec(services));
    reg.register(http_request::spec(services));
    reg.register(llm::ask_spec(services));
    reg.register(llm::decide_spec(services));
    reg.register(llm::extract_spec(services));
    reg.register(format::spec(services));
    reg.register(print::spec(services));
    round::register_all(reg);
    pick::register_all(reg);
    reg.register(schema::spec());
    // The report set: pure, and the only write goes through the host's own ValueIo.
    report::register_all(reg);

    reg.register(table::append::spec(services));
    reg.register(table::clear::spec(services));
    reg.register(table::count::spec(services));
    reg.register(table::find::spec(services));
    reg.register(table::read::spec(services));
    reg.register(table::set_cell::spec(services));
    reg.register(variables::get_spec(services));
    reg.register(variables::set_spec(services));
    reg.register(wait::spec(services));
}
