// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! What flows through a graph, and what the things in it are called.
//!
//! - [`id`] — the names. A node kind and a port are open strings, not enums, which is what lets
//!   one engine serve products with entirely different vocabularies.
//! - [`port`] — a pin on a node: its name, its type, and whether two of them may be connected.
//!   Refusing an impossible wire while it is being drawn is worth more than any error message
//!   afterwards.
//! - [`value`] — what actually travels. A typed enum, with an escape hatch (`Extern`) for the
//!   things a product knows about and the engine never will.
//! - [`codec`] — turning those values into something storable and back. Tagged, so a value that
//!   comes back knows what it was.

pub mod codec;
pub mod id;
pub mod port;
pub mod schema;
pub mod value;
