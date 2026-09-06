// SPDX-FileCopyrightText: 2026 amurcanov
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

use anyhow::{Result, bail};
use std::fs::File;
use tokio_util::sync::CancellationToken;

// ===== Stub implementation for all platforms (including iOS) =====
// The real TUN FD transport is only available on Android.
// This stub exists only to satisfy compilation.

pub struct FdReceiver;

impl FdReceiver {
    pub fn bind(_name: &str) -> Result<Self> {
        crate::log_error!("[TUN] Switching to proxy");
        Ok(Self)
    }

    pub async fn receive(&self, _cancel: &CancellationToken) -> Result<File> {
        crate::log_error!("[TUN] Switching to proxy");
        bail!("TUN FD is only supported on Android")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tun_is_stubbed_on_non_android() {
        // No-op test to satisfy compilation
        assert!(true);
    }
}