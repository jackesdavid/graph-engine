// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Jackes David Lemos

//! Whether an operation is worth trying again, and the error that carries the answer.

/// Whether the same call might work if it were tried again.
///
/// This exists so retry is a decision somebody made rather than a guess. Without it a retry
/// policy has to match on error text, which is how "connection refused" gets retried forever
/// alongside "that table does not exist".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Retry {
    /// The world was busy, unreachable or slow. Trying again later is reasonable.
    #[default]
    Maybe,
    /// It will fail the same way every time — a missing table, a malformed request, a
    /// credential that is wrong rather than expired. Retrying only delays the report.
    Never,
}

/// Something the world refused to do. The engine surfaces these; it does not interpret them.
///
/// Defaults to [`Retry::Maybe`], because a *host* failure is usually about the world rather
/// than about the request. A host that knows better says so with [`HostError::permanent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub retry: Retry,
    pub message: String,
}

impl HostError {
    pub fn new(message: impl Into<String>) -> Self {
        HostError {
            retry: Retry::Maybe,
            message: message.into(),
        }
    }

    /// It will fail the same way next time.
    pub fn permanent(message: impl Into<String>) -> Self {
        HostError {
            retry: Retry::Never,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

impl From<String> for HostError {
    fn from(s: String) -> Self {
        HostError::new(s)
    }
}

impl From<&str> for HostError {
    fn from(s: &str) -> Self {
        HostError::new(s)
    }
}
