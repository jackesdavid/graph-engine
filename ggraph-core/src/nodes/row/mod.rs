// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Working with the rows of a table that is on a wire.
//!
//! Distinct from [`table`](super::table), which is about tables that are STORED — named, outliving
//! the run, reached through a store. These take a `table` value as it travels between two nodes and
//! take it apart: all the rows, the first one, one cell of one.
//!
//! # The family
//!
//! ```text
//! table        { columns: [TableColumn], rows: [TableRow] }
//! table_rows   [TableRow]
//! table_row    { "document": "a.pdf", "score": 0.03 }
//! table_column { name: "score", type: "num" }
//! ```
//!
//! Four types rather than one, so a wire is refused while it is being drawn: a row does not go
//! where a table goes, and the rows on their own cannot answer what the columns are.
//!
//! # The schema is how a cell keeps its type
//!
//! Reading a cell by name could return anything, and a port that says `any` is a port the editor
//! cannot check. So the cell node is given the schema, and its output port carries the type THAT
//! column was declared as — chosen from a list of the column names rather than typed from memory.

mod cell;
mod first;
mod rows;

use crate::host::Host;
use crate::registry::NodeRegistry;

pub fn register_all<H: Host>(reg: &mut NodeRegistry<H>) {
    reg.register(rows::spec());
    reg.register(first::spec());
    reg.register(cell::spec());
}
