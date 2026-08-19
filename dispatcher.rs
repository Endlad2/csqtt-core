// SPDX-FileCopyrightText: 2026 amurcanov
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

use crate::{
    packet::{PacketBuf, PacketPool},
    stats::Stats,
    striped_scheduler::{DispatchTicket, PacketClass, StripedScheduler},
    tun,
};
use anyhow::Result;
use arc_swap::ArcSwap;
use crossbeam_queue::ArrayQueue;
use socket2::SockRef;
use std::{
    future::Future,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::UdpSocket, sync::Notify, task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
use std::fs::File;

const RETURN_CAPACITY: usize = 1024;
const RETURN_MAX_AGE: Duration = Duration::from_millis(250);

const QUEUE_ACTIVE: u64 = 1;

struct QueuedPacket {
    packet: PacketBuf,
    queued_at: Instant,
    epoch: u64,
}

struct PacketQueue {
    queue: ArrayQueue<QueuedPacket>,
    notify: Notify,
    state: AtomicU64,
    senders: AtomicUsize,
    receiver_open: AtomicBool,
    max_age: Duration,
}

pub struct PacketSender {
    shared: Arc<PacketQueue>,
}

pub struct PacketReceiver {
    shared: Arc<PacketQueue>,
}

impl Clone for PacketSender {
    fn clone(&self) -> Self {
        self.shared.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for PacketSender {
    fn drop(&mut self) {
        if self.shared.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.shared.notify.notify_waiters();
        }
    }
}

impl PacketSender {
    pub fn try_send(&self, packet: PacketBuf) -> std::result::Result<(), PacketBuf> {
        self.send(packet, false)
    }

    pub fn force_send(&self, packet: PacketBuf) -> std::result::Result<(), PacketBuf> {
        self.send(packet, true)
    }

    fn send(&self, packet: PacketBuf, force: bool) -> std::result::Result<(), PacketBuf> {
        if !self.shared.receiver_open.load(Ordering::Acquire) {
            return Err(packet);
        }
        let state = self.shared.state.load(Ordering::Acquire);
        if state & QUEUE_ACTIVE == 0 {
            return Err(packet);
        }
        let queued = QueuedPacket {
            packet,
            queued_at: Instant::now(),
            epoch: state >> 1,
        };
        if force {
            drop(self.shared.queue.force_push(queued));
        } else if let Err(queued) = self.shared.queue.push(queued) {
            return Err(queued.packet);
        }
        self.shared.notify.notify_one();
        Ok(())
    }
}

impl PacketReceiver {
    pub fn try_recv(&self) -> Option<PacketBuf> {
        loop {
            let queued = self.shared.queue.pop()?;
            let state = self.shared.state.load(Ordering::Acquire);
            if state & QUEUE_ACTIVE != 0
                && queued.epoch == state >> 1
                && Instant::now().saturating_duration_since(queued.queued_at) <= self.shared.max_age
            {
                return Some(queued.packet);
            }
        }
    }

    pub async fn recv(&self, cancel: &CancellationToken) -> Option<PacketBuf> {
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            if let Some(packet) = self.try_recv() {
                return Some(packet);
            }
            if self.shared.senders.load(Ordering::Acquire) == 0 {
                return None;
            }
            let notified = self.shared.notify.notified();
            if let Some(packet) = self.try_recv() {
                return Some(packet);
            }
            if self.shared.senders.load(Ordering::Acquire) == 0 {
                return None;
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return None,
                _ = notified => {}
            }
        }
    }

    fn resume(&self) {
        let previous = self.shared.state.load(Ordering::Acquire) >> 1;
        let epoch = previous.saturating_add(1);
        self.shared.state.store(epoch << 1, Ordering::Release);
        self.purge();
        self.shared
            .state
            .store((epoch << 1) | QUEUE_ACTIVE, Ordering::Release);
        self.shared.notify.notify_waiters();
    }

    fn suspend(&self) {
        let state = self.shared.state.load(Ordering::Acquire);
        self.shared
            .state
            .store(state & !QUEUE_ACTIVE, Ordering::Release);
        self.purge();
        self.shared.notify.notify_waiters();
    }

    fn purge(&self) {
        while self.shared.queue.pop().is_some() {}
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.shared.senders.load(Ordering::Acquire) == 0 && self.shared.queue.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.shared.queue.len()
    }
}

impl Drop for PacketReceiver {
    fn drop(&mut self) {
        self.shared.receiver_open.store(false, Ordering::Release);
        self.purge();
        self.shared.notify.notify_waiters();
    }
}

pub fn packet_channel(
    capacity: usize,
    max_age: Duration,
    active: bool,
) -> (PacketSender, PacketReceiver) {
    let state = u64::from(active) * QUEUE_ACTIVE;
    let shared = Arc::new(PacketQueue {
        queue: ArrayQueue::new(capacity.max(1)),
        notify: Notify::new(),
        state: AtomicU64::new(state),
        senders: AtomicUsize::new(1),
        receiver_open: AtomicBool::new(true),
        max_age,
    });
    (
        PacketSender {
            shared: shared.clone(),
        },
        PacketReceiver { shared },
    )
}

#[derive(Clone)]
pub struct WorkerChannels {
    pub id: usize,
    pub incarnation_id: u64,
    pub normal: PacketSender,
    pub small: PacketSender,
    pub bulk: PacketSender,
    pub doomsday: PacketSender,
}

pub struct Dispatcher {
    workers: ArcSwap<Vec<WorkerChannels>>,
    return_tx: PacketSender,
    scheduler: StripedScheduler,
    cancel: CancellationToken,
    tasks: tokio::sync::Mutex<Vec<JoinHandle<()>>>,
    tun_config: tokio::sync::Mutex<Option<(String, String, String)>>,
}

impl Dispatcher {
    pub async fn start(
        listen: &str,
        tun_uds: Option<String>,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
        cancel: CancellationToken,
    ) -> Result<(Arc<Self>, String)> {
        let tun_mode = tun_uds.is_some();
        let (return_tx, return_rx) = packet_channel(RETURN_CAPACITY, RETURN_MAX_AGE, !tun_mode);
        let dispatcher = Arc::new(Self {
            workers: ArcSwap::from_pointee(Vec::new()),
            return_tx,
            scheduler: StripedScheduler::new(),
            cancel: cancel.clone(),
            tasks: tokio::sync::Mutex::new(Vec::new()),
            tun_config: tokio::sync::Mutex::new(None),
        });

        if let Some(ref name) = tun_uds {
            #[cfg(unix)]
            {
                let name = name.clone();
                crate::log_error!("[КЛИЕНТ] Запуск UDS-слушателя: {name} для получения TUN FD...");
                let io_dispatcher = dispatcher.clone();
                let task_cancel = dispatcher.cancel.clone();
                let io_task = spawn_critical("TUN dispatcher", task_cancel, async move {
                    let mut return_rx = return_rx;
                    loop {
                        let result = tun::receive_fd(name.clone(), io_dispatcher.cancel.clone()).await;
                        let file = match result {
                            Ok(file) => file,
                            Err(_) if io_dispatcher.cancel.is_cancelled() => return,
                            Err(error) => {
                                crate::log_error!(
                                    "[ОШИБКА] Не удалось получить TUN FD из UDS: {error}"
                                );
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                continue;
                            }
                        };
                        crate::log_error!("[КЛИЕНТ] TUN FD успешно получен!");
                        return_rx.resume();
                        io_dispatcher
                            .run_tun_unix(file, &mut return_rx, pool.clone(), stats.clone())
                            .await;
                        return_rx.suspend();
                    }
                });
                dispatcher.tasks.lock().await.push(io_task);
            }

            #[cfg(windows)]
            {
                crate::log_error!("[КЛИЕНТ] Windows: создание TUN через wintun...");
                let device = tun::create_tun_device("csqtt1").await?;
                let io_dispatcher = dispatcher.clone();
                let task_cancel = dispatcher.cancel.clone();
                let io_task = spawn_critical("TUN dispatcher", task_cancel, async move {
                    let return_rx = return_rx;
                    return_rx.resume();
                    io_dispatcher
                        .run_tun_windows(device, return_rx, pool.clone(), stats.clone())
                        .await;
                });
                dispatcher.tasks.lock().await.push(io_task);
            }

            #[cfg(target_os = "macos")]
            {
                crate::log_error!("[КЛИЕНТ] macOS: создание TUN напрямую...");
                let file = tun::create_tun_device("csqtt1").await?;
                let io_dispatcher = dispatcher.clone();
                let task_cancel = dispatcher.cancel.clone();
                let io_task = spawn_critical("TUN dispatcher", task_cancel, async move {
                    let mut return_rx = return_rx;
                    return_rx.resume();
                    io_dispatcher
                        .run_tun_macos(file, &mut return_rx, pool.clone(), stats.clone())
                        .await;
                    return_rx.suspend();
                });
                dispatcher.tasks.lock().await.push(io_task);
            }

            #[cfg(not(any(unix, windows, target_os = "macos")))]
            {
                crate::log_error!("[ОШИБКА] TUN не поддерживается на этой ОС");
                return Err(anyhow::anyhow!("TUN not supported on this OS"));
            }

            Ok((dispatcher, "0".to_owned()))
        } else {
            let socket = bind_udp(listen).await?;
            let local_port = socket.local_addr()?.port().to_string();
            let socket = Arc::new(socket);
            let client = Arc::new(tokio::sync::RwLock::new(None));
            let read_dispatcher = dispatcher.clone();
            let read_socket = socket.clone();
            let read_client = client.clone();
            let read_pool = pool.clone();
            let read_stats = stats.clone();
            let read_cancel = dispatcher.cancel.clone();
            let read_task = spawn_critical("UDP reader", read_cancel, async move {
                read_dispatcher
                    .read_udp(read_socket, read_client, read_pool, read_stats)
                    .await;
            });
            let write_dispatcher = dispatcher.clone();
            let write_cancel = dispatcher.cancel.clone();
            let write_task = spawn_critical("UDP writer", write_cancel, async move {
                write_dispatcher
                    .write_udp(socket, client, return_rx, stats)
                    .await;
            });
            dispatcher
                .tasks
                .lock()
                .await
                .extend([read_task, write_task]);
            Ok((dispatcher, local_port))
        }
    }

    pub fn register(&self, channels: WorkerChannels) {
        let id = channels.id;
        self.workers.rcu(|workers| {
            let mut updated = (**workers).clone();
            updated.retain(|worker| worker.id != id);
            updated.push(channels.clone());
            updated.sort_unstable_by_key(|worker| worker.id);
            Arc::new(updated)
        });
    }

    pub fn unregister(&self, id: usize, incarnation_id: u64) {
        self.workers.rcu(|workers| {
            let mut updated = (**workers).clone();
            updated.retain(|worker| worker.id != id || worker.incarnation_id != incarnation_id);
            Arc::new(updated)
        });
    }

    #[cfg(test)]
    pub fn active_count(&self) -> usize {
        self.workers.load().len()
    }

    #[cfg(test)]
    pub fn worker(&self, id: usize) -> Option<WorkerChannels> {
        self.workers
            .load()
            .iter()
            .find(|worker| worker.id == id)
            .cloned()
    }

    pub fn return_packet(&self, packet: PacketBuf) {
        let _ = self.return_tx.force_send(packet);
    }

    pub async fn configure_tun(&self, ip: &str, dns: &str, gateway: &str) {
        let mut config = self.tun_config.lock().await;
        #[cfg(windows)]
        {
            let changed = config
                .as_ref()
                .map(|(old_ip, old_dns, old_gw)| {
                    old_ip != ip || old_dns != dns || old_gw != gateway
                })
                .unwrap_or(true);
            if changed {
                if let Err(error) = crate::tun::configure_windows_tun("csqtt1", ip, dns, gateway) {
                    crate::log_error!("[ОШИБКА] Не удалось настроить TUN: {error}");
                }
            }
        }
        *config = Some((ip.to_owned(), dns.to_owned(), gateway.to_owned()));
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        for task in self.tasks.lock().await.drain(..) {
            let _ = task.await;
        }
    }

    #[cfg(unix)]
    async fn run_tun_unix(
        self: &Arc<Self>,
        file: File,
        receiver: &mut PacketReceiver,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        use tokio::io::unix::AsyncFd;

        let device = match AsyncFd::new(file) {
            Ok(device) => Arc::new(device),
            Err(error) => {
                crate::log_error!("[ОШИБКА] Не удалось зарегистрировать TUN FD: {error}");
                return;
            }
        };

        let self_clone = self.clone();
        let pool_clone = pool.clone();
        let stats_clone = stats.clone();
        let device_clone = device.clone();
        let mut read_handle = tokio::spawn(async move {
            self_clone.read_tun_unix_async(device_clone, pool_clone, stats_clone).await
                .unwrap_or_else(|e| crate::log_error!("[ОШИБКА] Чтение TUN: {e}"));
        });

        let receiver = std::mem::replace(receiver, PacketReceiver {
            shared: receiver.shared.clone(),
        });
        let self_clone = self.clone();
        let stats_clone = stats.clone();
        let mut write_handle = tokio::spawn(async move {
            self_clone.write_tun_unix_async(device, receiver, stats_clone).await
                .unwrap_or_else(|e| crate::log_error!("[ОШИБКА] Запись TUN: {e}"));
        });

        tokio::select! {
            _ = self.cancel.cancelled() => {
                read_handle.abort();
                write_handle.abort();
            }
            _ = &mut read_handle => {
                write_handle.abort();
            }
            _ = &mut write_handle => {
                read_handle.abort();
            }
        }
    }

    #[cfg(unix)]
    async fn read_tun_unix_async(
        self: Arc<Self>,
        device: Arc<tokio::io::unix::AsyncFd<File>>,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) -> Result<(), std::io::Error> {
        use std::os::fd::AsRawFd;

        let mut burst = 0usize;
        let mut buf = vec![0u8; 4096];

        loop {
            if self.cancel.is_cancelled() {
                return Ok(());
            }

            let result = {
                let mut guard = device.readable().await?;
                guard.try_io(|inner| {
                    let length = unsafe {
                        libc::read(
                            inner.get_ref().as_raw_fd(),
                            buf.as_mut_ptr().cast(),
                            buf.len(),
                        )
                    };
                    if length < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(length as usize)
                    }
                })
            };

            match result {
                Ok(Ok(length)) if length > 0 => {
                    burst += 1;
                    let Some(mut packet) = pool.try_acquire() else {
                        continue;
                    };
                    if packet.set_read_len(length).is_err() {
                        continue;
                    }
                    packet.as_mut_slice().copy_from_slice(&buf[..length]);
                    stats
                        .total_bytes_up
                        .fetch_add(length as i64, Ordering::Relaxed);
                    self.dispatch(packet);

                    if burst >= 32 {
                        burst = 0;
                        tokio::task::yield_now().await;
                    }
                }
                Ok(Ok(0)) => {
                    crate::log_error!("[TUN] Конец файла, ожидаем новый FD");
                    return Ok(());
                }
                Ok(Ok(_)) => {
                    crate::log_error!("[TUN] Некорректная длина чтения");
                    return Ok(());
                }
                Ok(Err(e)) if is_retryable_tun_error(&e) => {
                    tokio::task::yield_now().await;
                }
                Ok(Err(e)) if is_closed_tun_error(&e) => {
                    crate::log_error!("[TUN] Интерфейс закрыт, ожидаем новый FD");
                    return Ok(());
                }
                Ok(Err(e)) => {
                    crate::log_error!("[ОШИБКА] Чтение TUN завершено: {e}");
                    return Err(e);
                }
                Err(_) => {
                    tokio::task::yield_now().await;
                }
            }
        }
    }

    #[cfg(unix)]
    async fn write_tun_unix_async(
        self: Arc<Self>,
        device: Arc<tokio::io::unix::AsyncFd<File>>,
        receiver: PacketReceiver,
        stats: Arc<Stats>,
    ) -> Result<(), std::io::Error> {
        use std::os::fd::AsRawFd;

        let mut burst = 0usize;

        loop {
            let packet = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Ok(()),
                packet = receiver.recv(&self.cancel) => match packet {
                    Some(packet) => packet,
                    None => return Ok(()),
                },
            };

            let result = {
                let mut guard = device.writable().await?;
                guard.try_io(|inner| {
                    let remaining = packet.as_slice();
                    let length = unsafe {
                        libc::write(
                            inner.get_ref().as_raw_fd(),
                            remaining.as_ptr().cast(),
                            remaining.len(),
                        )
                    };
                    if length < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(length as usize)
                    }
                })
            };

            match result {
                Ok(Ok(length)) => {
                    if length != packet.len() {
                        crate::log_error!("[TUN] Записано {} байт из {}", length, packet.len());
                    }
                    stats
                        .total_bytes_down
                        .fetch_add(length as i64, Ordering::Relaxed);

                    burst += 1;
                    if burst >= 64 {
                        burst = 0;
                        tokio::task::yield_now().await;
                    }
                }
                Ok(Err(e)) if is_retryable_tun_error(&e) => {
                    tokio::task::yield_now().await;
                }
                Ok(Err(e)) if is_closed_tun_error(&e) => {
                    crate::log_error!("[TUN] Интерфейс закрыт, ожидаем новый FD");
                    return Ok(());
                }
                Ok(Err(e)) => {
                    crate::log_error!("[ОШИБКА] Запись TUN завершена: {e}");
                    return Err(e);
                }
                Err(_) => {
                    tokio::task::yield_now().await;
                }
            }
        }
    }

    #[cfg(windows)]
    async fn run_tun_windows(
        self: &Arc<Self>,
        device: ::tun::Device,
        receiver: PacketReceiver,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        let device = Arc::new(tokio::sync::Mutex::new(device));
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));

        let self_clone = self.clone();
        let pool_clone = pool.clone();
        let stats_clone = stats.clone();
        let device_clone = device.clone();
        let mut read_handle = tokio::spawn(async move {
            self_clone.read_tun_windows_blocking(device_clone, pool_clone, stats_clone).await;
        });

        let self_clone = self.clone();
        let stats_clone = stats.clone();
        let mut write_handle = tokio::spawn(async move {
            self_clone.write_tun_windows_blocking(device, receiver, stats_clone).await;
        });

        tokio::select! {
            _ = self.cancel.cancelled() => {
                read_handle.abort();
                write_handle.abort();
            }
            _ = &mut read_handle => {
                write_handle.abort();
            }
            _ = &mut write_handle => {
                read_handle.abort();
            }
        }
    }

    #[cfg(windows)]
    async fn read_tun_windows_blocking(
        self: Arc<Self>,
        device: Arc<tokio::sync::Mutex<::tun::Device>>,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        use std::io::Read;

        loop {
            if self.cancel.is_cancelled() {
                return;
            }

            let device_clone = device.clone();
            let result = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, std::io::Error> {
                let mut device = device_clone.blocking_lock();
                let mut buf = vec![0u8; 4096];
                let n = device.read(&mut buf)?;
                if n == 0 {
                    return Ok(Vec::new());
                }
                Ok(buf[..n].to_vec())
            }).await;

            match result {
                Ok(Ok(data)) if data.is_empty() => {
                    crate::log_error!("[TUN] Конец файла");
                    return;
                }
                Ok(Ok(data)) => {
                    let Some(mut packet) = pool.try_acquire() else {
                        continue;
                    };
                    if packet.set_read_len(data.len()).is_err() {
                        continue;
                    }
                    packet.as_mut_slice().copy_from_slice(&data);
                    stats
                        .total_bytes_up
                        .fetch_add(data.len() as i64, Ordering::Relaxed);
                    self.dispatch(packet);
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::task::yield_now().await;
                }
                Ok(Err(e)) => {
                    crate::log_error!("[ОШИБКА] Чтение TUN на Windows: {e}");
                    return;
                }
                Err(e) => {
                    crate::log_error!("[ОШИБКА] Задача чтения TUN на Windows: {e}");
                    return;
                }
            }
        }
    }

    #[cfg(windows)]
    async fn write_tun_windows_blocking(
        self: Arc<Self>,
        device: Arc<tokio::sync::Mutex<::tun::Device>>,
        receiver: Arc<tokio::sync::Mutex<PacketReceiver>>,
        stats: Arc<Stats>,
    ) {
        use std::io::Write;

        loop {
            let packet = {
                let receiver_guard = receiver.lock().await;
                tokio::select! {
                    biased;
                    _ = self.cancel.cancelled() => return,
                    packet = receiver_guard.recv(&self.cancel) => match packet {
                        Some(packet) => packet,
                        None => return,
                    },
                }
            };

            let data = packet.as_slice().to_vec();
            let device_clone = device.clone();
            let result = tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
                let mut device = device_clone.blocking_lock();
                device.write_all(&data)?;
                Ok(())
            }).await;

            match result {
                Ok(Ok(())) => {
                    stats
                        .total_bytes_down
                        .fetch_add(packet.len() as i64, Ordering::Relaxed);
                }
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::task::yield_now().await;
                }
                Ok(Err(e)) => {
                    crate::log_error!("[ОШИБКА] Запись TUN на Windows: {e}");
                    return;
                }
                Err(e) => {
                    crate::log_error!("[ОШИБКА] Задача записи TUN на Windows: {e}");
                    return;
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    async fn run_tun_macos(
        self: &Arc<Self>,
        file: File,
        receiver: &mut PacketReceiver,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        use tokio::io::unix::AsyncFd;

        let device = match AsyncFd::new(file) {
            Ok(device) => Arc::new(device),
            Err(error) => {
                crate::log_error!("[ОШИБКА] Не удалось зарегистрировать TUN FD: {error}");
                return;
            }
        };

        let self_clone = self.clone();
        let pool_clone = pool.clone();
        let stats_clone = stats.clone();
        let device_clone = device.clone();
        let mut read_handle = tokio::spawn(async move {
            self_clone.read_tun_unix_async(device_clone, pool_clone, stats_clone).await
                .unwrap_or_else(|e| crate::log_error!("[ОШИБКА] Чтение TUN: {e}"));
        });

        let receiver = std::mem::replace(receiver, PacketReceiver {
            shared: receiver.shared.clone(),
        });
        let self_clone = self.clone();
        let stats_clone = stats.clone();
        let mut write_handle = tokio::spawn(async move {
            self_clone.write_tun_unix_async(device, receiver, stats_clone).await
                .unwrap_or_else(|e| crate::log_error!("[ОШИБКА] Запись TUN: {e}"));
        });

        tokio::select! {
            _ = self.cancel.cancelled() => {
                read_handle.abort();
                write_handle.abort();
            }
            _ = &mut read_handle => {
                write_handle.abort();
            }
            _ = &mut write_handle => {
                read_handle.abort();
            }
        }
    }

    #[cfg(not(any(unix, windows, target_os = "macos")))]
    async fn run_tun(
        self: &Arc<Self>,
        _file: File,
        _receiver: &mut PacketReceiver,
        _pool: Arc<PacketPool>,
        _stats: Arc<Stats>,
    ) {
        crate::log_error!("[ОШИБКА] TUN не поддерживается на этой ОС");
    }

    async fn read_udp(
        self: Arc<Self>,
        socket: Arc<UdpSocket>,
        client: Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
        pool: Arc<PacketPool>,
        stats: Arc<Stats>,
    ) {
        loop {
            let Some(mut packet) = pool.try_acquire() else {
                tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
                continue;
            };
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                result = socket.recv_from(packet.read_area()) => match result {
                    Ok((length, address)) => {
                        *client.write().await = Some(address);
                        if packet.set_read_len(length).is_err() {
                            continue;
                        }
                        stats.total_bytes_up.fetch_add(length as i64, Ordering::Relaxed);
                        self.dispatch(packet);
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        }
    }

    fn dispatch(&self, packet: PacketBuf) {
        self.dispatch_now(packet);
    }

    fn dispatch_now(&self, mut packet: PacketBuf) {
        let workers = self.workers.load();
        let Some(ticket) = self.scheduler.begin(workers.len(), packet.as_slice()) else {
            return;
        };
        if let Err(returned) = try_workers(&workers, ticket, packet) {
            packet = returned;
            let _ = try_normal_workers(&workers, ticket, packet);
        }
    }

    async fn write_udp(
        &self,
        socket: Arc<UdpSocket>,
        client: Arc<tokio::sync::RwLock<Option<SocketAddr>>>,
        receiver: PacketReceiver,
        stats: Arc<Stats>,
    ) {
        loop {
            let packet = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return,
                packet = receiver.recv(&self.cancel) => match packet {
                    Some(packet) => packet,
                    None => return,
                },
            };
            let address = *client.read().await;
            self.write_udp_packet(&socket, address, &stats, packet)
                .await;
        }
    }

    async fn write_udp_packet(
        &self,
        socket: &UdpSocket,
        address: Option<SocketAddr>,
        stats: &Stats,
        packet: PacketBuf,
    ) {
        if let Some(address) = address
            && socket.send_to(packet.as_slice(), address).await.is_ok()
        {
            stats
                .total_bytes_down
                .fetch_add(packet.len() as i64, Ordering::Relaxed);
        }
    }
}

fn spawn_critical<F>(name: &'static str, cancel: CancellationToken, future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(error) = tokio::spawn(future).await {
            crate::log_error!("[СУПЕРВИЗОР] {name} завершился аварийно: {error}");
            cancel.cancel();
        }
    })
}

#[cfg(unix)]
fn is_closed_tun_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EIO || code == libc::EBADF || code == libc::ENODEV
    )
}

#[cfg(unix)]
fn is_retryable_tun_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::Interrupted
        || error.kind() == std::io::ErrorKind::WouldBlock
        || matches!(
            error.raw_os_error(),
            Some(code) if code == libc::ENOBUFS || code == libc::ENOMEM
        )
}

fn try_workers(
    workers: &[WorkerChannels],
    ticket: DispatchTicket,
    mut packet: PacketBuf,
) -> Result<(), PacketBuf> {
    for offset in 0..ticket.cohort_len {
        let index = ticket.worker_index(offset);
        let worker = &workers[index];
        let channel = match ticket.class {
            PacketClass::Doomsday => &worker.doomsday,
            PacketClass::Small => &worker.small,
            PacketClass::Bulk => &worker.bulk,
        };
        match channel.try_send(packet) {
            Ok(()) => return Ok(()),
            Err(returned) => packet = returned,
        }
    }
    Err(packet)
}

fn try_normal_workers(
    workers: &[WorkerChannels],
    ticket: DispatchTicket,
    mut packet: PacketBuf,
) -> Result<(), PacketBuf> {
    for offset in 0..ticket.cohort_len {
        let index = ticket.worker_index(offset);
        match workers[index].normal.try_send(packet) {
            Ok(()) => return Ok(()),
            Err(returned) => packet = returned,
        }
    }
    if workers.is_empty() || ticket.cohort_len == 0 {
        return Err(packet);
    }
    workers[ticket.worker_index(0)].normal.force_send(packet)
}

async fn bind_udp(address: &str) -> Result<UdpSocket> {
    for attempt in 1..=5 {
        match UdpSocket::bind(address).await {
            Ok(socket) => {
                SockRef::from(&socket).set_recv_buffer_size(625 * 1024)?;
                SockRef::from(&socket).set_send_buffer_size(625 * 1024)?;
                return Ok(socket);
            }
            Err(error) if attempt < 5 => {
                crate::log_error!("[ОЖИДАНИЕ] Порт {address} занят. Жду... ({attempt}/5): {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(error) => crate::log_error!("[АВТО-ПОРТ] Порт {address} всё ещё занят: {error}"),
        }
    }
    UdpSocket::bind("127.0.0.1:0").await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::VecDeque;

    #[derive(Clone, Copy, Default)]
    struct QueueCoverage {
        full_rejection: usize,
        forced_replacement: usize,
        inactive_rejection: usize,
        suspended: usize,
        resumed: usize,
        purged: usize,
    }

    impl QueueCoverage {
        fn complete(self) -> bool {
            self.full_rejection > 0
                && self.forced_replacement > 0
                && self.inactive_rejection > 0
                && self.suspended > 0
                && self.resumed > 0
                && self.purged > 0
        }
    }

    fn test_dispatcher() -> (Arc<Dispatcher>, PacketReceiver) {
        let (return_tx, return_rx)
