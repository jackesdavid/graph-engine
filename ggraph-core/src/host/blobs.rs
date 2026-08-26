// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where values too large to sit in a database column are kept.

use super::error::HostError;

/// Where values too large to inline are kept.
///
/// The codec calls this when an [`ExternValue`](crate::ExternValue) or a [`Bytes`](crate::Bytes)
/// needs to survive a restart. [`disabled`](Disabled) is a legitimate implementation: it reports
/// `enabled() == false` and the codec drops such values rather than failing the run.
pub trait ValueIo: Send + Sync {
    fn enabled(&self) -> bool;
    fn put(&self, bytes: &[u8], mime: &str) -> Result<String, HostError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, HostError>;
}

/// A blob store that isn't there. Values that need one are dropped, and the drop is recorded.
#[derive(Debug)]
pub struct Disabled;

impl ValueIo for Disabled {
    fn enabled(&self) -> bool {
        false
    }
    fn put(&self, _: &[u8], _: &str) -> Result<String, HostError> {
        Err(HostError::permanent("no blob store configured"))
    }
    fn get(&self, _: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError::permanent("no blob store configured"))
    }
}
