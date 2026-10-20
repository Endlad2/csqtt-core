use anyhow::{Result, bail};
use std::fs::File;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
pub struct FdReceiver;

#[cfg(unix)]
impl FdReceiver {
    pub fn bind(name: &str) -> Result<Self> {
        crate::log_info!("[PROXY] Используется прокси (abstract UDS: {name})");
        Ok(Self)
    }

    pub async fn receive(&self, cancel: &CancellationToken) -> Result<File> {
        crate::log_info!("[PROXY] Ожидание TUN FD через прокси...");
        tokio::select! {
            _ = cancel.cancelled() => bail!("TUN FD wait cancelled"),
            _ = tokio::time::sleep(std::time::Duration::from_secs(u64::MAX)) => {
                unreachable!()
            }
        }
    }
}

#[cfg(not(unix))]
pub struct FdReceiver;

#[cfg(not(unix))]
impl FdReceiver {
    pub fn bind(_name: &str) -> Result<Self> {
        bail!("TUN FD transport is available only on Android and Unix")
    }

    pub async fn receive(&self, _cancel: &CancellationToken) -> Result<File> {
        bail!("TUN FD transport is available only on Android and Unix")
    }
}