// SPDX-FileCopyrightText: 2026 amurcanov
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

use anyhow::{Result, bail};
use std::fs::File;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
pub async fn receive_fd(name: String, cancel: CancellationToken) -> Result<File> {
    tokio::task::spawn_blocking(move || receive_fd_blocking(&name, &cancel)).await?
}

#[cfg(unix)]
fn receive_fd_blocking(name: &str, cancel: &CancellationToken) -> Result<File> {
    use nix::{
        cmsg_space,
        errno::Errno,
        sys::socket::{
            AddressFamily, Backlog, ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr,
            accept, bind, listen, recvmsg, socket,
        },
    };
    use std::{
        io::IoSliceMut,
        os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        thread,
        time::Duration,
    };

    fn configure_nonblocking(descriptor: RawFd) -> Result<()> {
        let status = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if status == -1
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, status | libc::O_NONBLOCK) } == -1
            || unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    let listener = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        None,
    )?;
    bind(
        listener.as_raw_fd(),
        &UnixAddr::new_abstract(name.as_bytes())?,
    )?;
    listen(&listener, Backlog::new(1)?)?;

    let connection = loop {
        if cancel.is_cancelled() {
            bail!("TUN FD wait cancelled");
        }
        match accept(listener.as_raw_fd()) {
            Ok(descriptor) => {
                let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
                configure_nonblocking(descriptor.as_raw_fd())?;
                break descriptor;
            }
            Err(Errno::EAGAIN) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return Err(error.into()),
        }
    };

    let mut data = [0u8; 1];
    loop {
        if cancel.is_cancelled() {
            bail!("TUN FD receive cancelled");
        }
        let mut slices = [IoSliceMut::new(&mut data)];
        let mut ancillary = cmsg_space!([std::os::fd::RawFd; 1]);
        match recvmsg::<UnixAddr>(
            connection.as_raw_fd(),
            &mut slices,
            Some(&mut ancillary),
            MsgFlags::empty(),
        ) {
            Ok(message) => {
                if message.bytes == 0 {
                    bail!("TUN FD connection closed");
                }
                for control in message.cmsgs()? {
                    if let ControlMessageOwned::ScmRights(descriptors) = control
                        && let Some(descriptor) = descriptors.into_iter().next()
                    {
                        let file = unsafe { File::from_raw_fd(descriptor) };
                        configure_nonblocking(file.as_raw_fd())?;
                        return Ok(file);
                    }
                }
                bail!("no fd received");
            }
            Err(Errno::EAGAIN) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(windows)]
pub async fn receive_fd(name: String, cancel: CancellationToken) -> Result<File> {
    use std::os::windows::io::FromRawHandle;
    use tokio::io::AsyncReadExt;
    use tokio::net::windows::named_pipe::ClientOptions;

    let pipe_name = format!(r"\\.\pipe\{}", name);
    let mut pipe = ClientOptions::new()
        .open(&pipe_name)
        .map_err(|e| anyhow::anyhow!("Failed to open named pipe {}: {}", pipe_name, e))?;

    let mut handle_buf = [0u8; 8];
    tokio::select! {
        _ = cancel.cancelled() => bail!("TUN FD receive cancelled"),
        result = pipe.read_exact(&mut handle_buf) => {
            result.map_err(|e| anyhow::anyhow!("Failed to read TUN handle from pipe: {}", e))?;
        }
    }

    let handle = u64::from_le_bytes(handle_buf) as *mut std::ffi::c_void;
    let file = unsafe { File::from_raw_handle(handle) };

    Ok(file)
}

#[cfg(windows)]
pub async fn create_tun_device(name: &str) -> Result<::tun::Device> {
    use ::tun::Configuration;

    let mut config = Configuration::default();
    config.tun_name(name).up();

    let device = ::tun::create(&config)
        .map_err(|e| anyhow::anyhow!("Failed to create TUN device on Windows: {}", e))?;

    Ok(device)
}

#[cfg(windows)]
pub fn configure_windows_tun(adapter_name: &str, ip: &str, dns: &str, gateway: &str) -> Result<()> {
    // Если gateway = "0" или пустой, вычисляем из IP: 10.66.67.3 -> 10.66.67.1
    let gateway = if gateway.is_empty() || gateway == "0" {
        match ip.rsplit_once('.') {
            Some((prefix, _)) => format!("{prefix}.1"),
            None => bail!("Неверный формат IP: {ip}"),
        }
    } else {
        gateway.to_owned()
    };

    crate::log_error!(
        "[КЛИЕНТ] Настройка TUN: IP={ip}, DNS={dns}, Gateway={gateway} на {adapter_name}"
    );

    let status = std::process::Command::new("netsh")
        .args([
            "interface",
            "ip",
            "set",
            "address",
            &format!("name={adapter_name}"),
            "static",
            ip,
            "255.255.255.0",
            &gateway,
        ])
        .status()
        .map_err(|e| anyhow::anyhow!("Не удалось запустить netsh: {e}"))?;

    if !status.success() {
        bail!("netsh не смог установить IP {ip} на {adapter_name}");
    }
    crate::log_error!("[КЛИЕНТ] IP {ip} установлен на {adapter_name}");

    if !dns.is_empty() {
        let primary_dns = dns.split(',').next().unwrap_or("1.1.1.1");
        let status = std::process::Command::new("netsh")
            .args([
                "interface",
                "ip",
                "set",
                "dns",
                &format!("name={adapter_name}"),
                "static",
                primary_dns,
            ])
            .status()
            .map_err(|e| anyhow::anyhow!("Не удалось запустить netsh для DNS: {e}"))?;

        if !status.success() {
            crate::log_error!(
                "[КЛИЕНТ] Предупреждение: netsh не смог установить DNS {primary_dns}"
            );
        } else {
            crate::log_error!("[КЛИЕНТ] DNS {primary_dns} установлен на {adapter_name}");
        }
    }

    Ok(())
}

#[cfg(target_os = "macos")]
pub async fn create_tun_device(name: &str) -> Result<File> {
    use ::tun::Configuration;
    use std::os::fd::FromFd;

    let mut config = Configuration::default();
    config.tun_name(name).up();

    let device = ::tun::create(&config)
        .map_err(|e| anyhow::anyhow!("Failed to create TUN device on macOS: {}", e))?;

    let file = File::from(device);
    Ok(file)
}

#[cfg(all(not(unix), not(windows), not(target_os = "macos")))]
pub async fn receive_fd(_name: String, _cancel: CancellationToken) -> Result<File> {
    bail!("TUN FD transport is available only on Unix, Windows, and macOS")
}
