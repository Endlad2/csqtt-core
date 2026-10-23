//! Stub TUN FD transport.
//!
//! Эта версия модуля — минимальная заглушка, чтобы `csqtt-core` собирался
//! на всех платформах без реального UDS/SCM_RIGHTS-транспорта TUN-дескрипторов.
//!
//! Публичный API (`FdReceiver::bind` / `FdReceiver::receive`) сохранён, чтобы
//! `dispatcher.rs` и `main.rs` компилировались без изменений.
//!
//! Поведение:
//!   - `bind(name)` — всегда успешно возвращает объект-receiver, `name`
//!     игнорируется.
//!   - `receive(cancel)` — никогда не возвращает `File`, вечно ждёт
//!     `cancel.cancelled()` и затем завершается с ошибкой.
//!
//! В результате dispatcher при заданном `tun_uds` переходит в режим ожидания
//! и не падает; при отмене — корректно выходит.

use anyhow::{Result, bail};
use std::fs::File;
use tokio_util::sync::CancellationToken;

pub struct FdReceiver {
    #[allow(dead_code)]
    name: String,
}

impl FdReceiver {
    pub fn bind(name: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_owned(),
        })
    }

    pub async fn receive(&self, cancel: &CancellationToken) -> Result<File> {
        cancel.cancelled().await;
        bail!("FdReceiver is a stub: TUN FD transport is disabled")
    }
}
