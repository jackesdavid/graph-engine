// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! `table_clear` — empty a table.
//!
//! The only node in the standard set that destroys data on purpose, so it says which table in
//! its log line and refuses to run without one. There is no "clear the table this is wired to"
//! form: an emptied table because a wire carried the wrong name is not recoverable from a
//! canvas.

use super::table_name;
use crate::host::Host;
use crate::spec::{NodeCx, NodeError, NodeRun, NodeSpec, Timeout};
use crate::value::PortValues;
use serde_json::json;

struct Clear {
    tables: std::sync::Arc<dyn crate::nodes::services::TableStore>,
}

impl<H: Host> NodeRun<H> for Clear {
    fn run(&self, cx: &NodeCx<'_, H>) -> Result<PortValues, NodeError> {
        let table = table_name(cx.config).ok_or(NodeError::new("no table name"))?;
        self.tables.clear(&table)?;
        Ok(PortValues::new())
    }

    fn summary(&self, cx: &NodeCx<'_, H>, _out: &PortValues) -> String {
        format!("emptied {}", table_name(cx.config).unwrap_or_default())
    }
}

pub fn spec<H: Host>(services: &crate::nodes::services::Services) -> NodeSpec<H> {
    NodeSpec::effectful("table_clear", "Empty a Table", "Tables")
        .about(r#"Empties a stored table, keeping its columns.

For a table that is rebuilt each run rather than added to.

```
On schedule --> Empty a Table (results) --> For Each --item--> Add a Row
```"#)
        .with_config(|| json!({ "table": "" }))
        .with_timeout(Timeout::Secs(30))
        .running(Clear {
            tables: services.tables.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::testkit::TestHost;
    use crate::spec::Behavior;

    #[test]
    fn it_will_not_run_without_a_table_name() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        let Behavior::Run(r) = &s.behavior else {
            unreachable!()
        };
        let host = TestHost::new();
        let cfg = json!({ "table": "" });
        let inputs = PortValues::new();
        let cx = NodeCx {
            config: &cfg,
            inputs: &inputs,
            node: 1,
            host: &host,
            vars: Default::default(),
            declared_inputs: None,
        };
        assert!(r.run(&cx).is_err());
    }

    #[test]
    fn the_table_is_named_in_the_config_not_taken_from_a_wire() {
        let s: NodeSpec<TestHost> = spec(&crate::nodes::services::Services::none());
        assert!(
            s.inputs.resolve(&json!({})).is_empty(),
            "an emptied table because a wire carried the wrong name is not recoverable from a \
             canvas"
        );
    }
}
