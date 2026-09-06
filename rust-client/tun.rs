// SPDX-FileCopyrightText: 2026 amurcanov
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

// TUN FD transport is only available on Android.
// On other platforms, this module provides only a stub.

#[cfg(target_os = "android")]
mod android_impl {
    use anyhow::{Result, bail};
    use std::fs::File;
    use tokio_util::sync::CancellationToken;
    use nix::sys::socket::{
        AddressFamily, Backlog, SockFlag, SockType, UnixAddr, bind, listen, socket,
    };
    use std::os::fd::AsRawFd;

    pub struct FdReceiver {
        listener: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    }

    impl FdReceiver {
        pub fn bind(name: &str) -> Result<Self> {
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
            listen(&listener, Backlog::new(4)?)?;
            Ok(Self {
                listener: tokio::io::unix::AsyncFd::new(listener)?,
            })
        }

        pub async fn receive(&self, cancel: &CancellationToken) -> Result<File> {
            use nix::{
                cmsg_space,
                errno::Errno,
                sys::socket::{ControlMessageOwned, MsgFlags, accept, recvmsg, send},
            };
            use std::{
                io::IoSliceMut,
                os::fd::{FromRawFd, OwnedFd},
            };
            use tokio::io::unix::AsyncFd;

            let connection = loop {
                tokio::select! {
                    _ = cancel.cancelled() => bail!("TUN FD wait cancelled"),
                    ready = self.listener.readable() => {
                        let mut ready = ready?;
                        match accept(self.listener.get_ref().as_raw_fd()) {
                            Ok(descriptor) => {
                                let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
                                configure_nonblocking(descriptor.as_raw_fd())?;
                                break descriptor;
                            }
                            Err(Errno::EAGAIN) => ready.clear_ready(),
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
            };

            let connection = AsyncFd::new(connection)?;
            let mut data = [0u8; 1];
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => bail!("TUN FD receive cancelled"),
                    ready = connection.readable() => {
                        let mut ready = ready?;
                        let mut slices = [IoSliceMut::new(&mut data)];
                        let mut ancillary = cmsg_space!([std::os::fd::RawFd; 1]);
                        match recvmsg::<nix::sys::socket::UnixAddr>(
                            connection.get_ref().as_raw_fd(),
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
                                        let _ = send(
                                            connection.get_ref().as_raw_fd(),
                                            &[1],
                                            MsgFlags::MSG_NOSIGNAL,
                                        );
                                        return Ok(file);
                                    }
                                }
                                bail!("no fd received");
                            }
                            Err(Errno::EAGAIN) => ready.clear_ready(),
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
            }
        }
    }

    fn configure_nonblocking(descriptor: std::os::fd::RawFd) -> Result<()> {
        let status = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if status == -1
            || unsafe { libc::fcntl(descriptor, libc::F_SETFL, status | libc::O_NONBLOCK) } == -1
            || unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::FdReceiver;
        use nix::sys::socket::{
            AddressFamily, ControlMessage, MsgFlags, SockFlag, SockType, UnixAddr, connect, sendmsg,
            socket,
        };
        use std::{
            fs::File,
            io::IoSlice,
            os::{
                fd::{AsRawFd, OwnedFd},
                unix::fs::FileTypeExt,
            },
        };
        use tokio_util::sync::CancellationToken;

        fn send_fd(name: &str, file: &File) -> OwnedFd {
            let socket = socket(
                AddressFamily::Unix,
                SockType::Stream,
                SockFlag::SOCK_CLOEXEC,
                None,
            )
            .unwrap();
            connect(
                socket.as_raw_fd(),
                &UnixAddr::new_abstract(name.as_bytes()).unwrap(),
            )
            .unwrap();
            let payload = [1u8];
            let slices = [IoSlice::new(&payload)];
            let controls = [ControlMessage::ScmRights(&[file.as_raw_fd()])];
            sendmsg::<UnixAddr>(
                socket.as_raw_fd(),
                &slices,
                &controls,
                MsgFlags::empty(),
                None,
            )
            .unwrap();
            socket
        }

        fn receive_ack(socket: &OwnedFd) -> u8 {
            let mut ack = [0u8; 1];
            let received =
                unsafe { libc::recv(socket.as_raw_fd(), ack.as_mut_ptr().cast(), ack.len(), 0) };
            assert_eq!(received, 1);
            ack[0]
        }

        #[tokio::test]
        async fn persistent_receiver_accepts_a_replacement_fd() {
            let name = format!(
                "csqtt-tun-test-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            );
            let receiver = FdReceiver::bind(&name).unwrap();
            let cancel = CancellationToken::new();
            let first_source = File::open("/dev/null").unwrap();
            let second_source = File::open("/dev/null").unwrap();

            let first_socket = send_fd(&name, &first_source);
            let first = receiver.receive(&cancel).await.unwrap();
            assert_eq!(receive_ack(&first_socket), 1);
            let second_socket = send_fd(&name, &second_source);
            let second = receiver.receive(&cancel).await.unwrap();
            assert_eq!(receive_ack(&second_socket), 1);

            assert!(first.metadata().unwrap().file_type().is_char_device());
            assert!(second.metadata().unwrap().file_type().is_char_device());
        }
    }
}

#[cfg(target_os = "android")]
pub use android_impl::FdReceiver;

// ===== Non-Android stub =====

#[cfg(not(target_os = "android"))]
pub struct FdReceiver;

#[cfg(not(target_os = "android"))]
impl FdReceiver {
    pub fn bind(_name: &str) -> anyhow::Result<Self> {
        crate::log_error!("[TUN] Switching to proxy");
        Ok(Self)
    }

    pub async fn receive(&self, _cancel: &CancellationToken) -> anyhow::Result<std::fs::File> {
        crate::log_error!("[TUN] Switching to proxy");
        anyhow::bail!("TUN FD is only supported on Android")
    }
}
