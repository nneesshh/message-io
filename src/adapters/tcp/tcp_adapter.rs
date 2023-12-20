#[cfg(unix)]
use std::ffi::CString;
use std::io::{self, ErrorKind, Read, Write};
use std::mem::forget;
use std::net::SocketAddr;
#[cfg(target_os = "macos")]
use std::num::NonZeroU32;
use std::ops::Deref;
use std::os::raw::c_int;
#[cfg(target_os = "windows")]
use std::os::windows::io::{AsRawSocket, FromRawSocket};
#[cfg(not(target_os = "windows"))]
use std::os::{fd::AsRawFd, unix::io::FromRawFd};

use mio::event::Source;
use mio::net::{TcpListener, TcpStream};
use net_packet::{take_small_packet, NetPacketGuard};
pub use socket2::TcpKeepalive;

use socket2::{Domain, Protocol, Socket, Type};

use crate::network::adapter::{
    AcceptedType, Adapter, ListeningInfo, Local, PendingStatus, ReadStatus, Remote, Resource,
    SendStatus,
};
use crate::network::driver::ListenConfig;

/// The maximum length of the pending (unaccepted) connection queue of a listener.
pub const LISTENER_BACKLOG: c_int = 1024;

/// Plain struct used as a returned value of [`Remote::connect_with()`]
pub struct TcpConnectionInfo {
    /// The new created remote resource
    pub stream: TcpStream,

    /// Local address of the interal resource used.
    pub local_addr: SocketAddr,

    /// Peer address of the interal resource used.
    pub peer_addr: SocketAddr,

    ///
    pub keepalive_opt: Option<TcpKeepalive>,
}

///
pub struct TcpAcceptPayload {
    pub keepalive_opt: Option<TcpKeepalive>,
}

///
pub struct TcpConnectPayload {
    pub keepalive_opt: Option<TcpKeepalive>,
}

///
pub(crate) struct TcpAdapter;

impl Adapter for TcpAdapter {
    type Remote = RemoteResource;
    type Local = LocalResource;
}

///
pub(crate) struct RemoteResource {
    pub stream: TcpStream,
    pub keepalive_opt: Option<TcpKeepalive>,
}

impl Resource for RemoteResource {
    #[inline(always)]
    fn source(&mut self) -> &mut dyn Source {
        &mut self.stream
    }
}

impl Remote for RemoteResource {
    #[inline(always)]
    fn receive(&self, process_data: &mut dyn FnMut(NetPacketGuard)) -> ReadStatus {
        loop {
            let mut input_buffer = take_small_packet();
            let buf = input_buffer.as_write_mut();

            let stream = &self.stream;
            match stream.deref().read(buf) {
                Ok(0) => break ReadStatus::Disconnected,
                Ok(size) => {
                    //
                    input_buffer.end_write(size);
                    process_data(input_buffer)
                }
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(ref err) if err.kind() == ErrorKind::WouldBlock => {
                    break ReadStatus::WaitNextEvent
                }
                Err(ref err) if err.kind() == ErrorKind::ConnectionReset => {
                    break ReadStatus::Disconnected
                }
                Err(err) => {
                    log::error!("TCP receive error: {}", err);
                    break ReadStatus::Disconnected; // should not happen
                }
            }
        }
    }

    #[inline(always)]
    fn send(&self, data: &[u8]) -> SendStatus {
        // TODO: The current implementation implies an active waiting,
        // improve it using POLLIN instead to avoid active waiting.
        // Note: Despite the fear that an active waiting could generate,
        // this only occurs in the case when the receiver is full because reads slower than it sends.
        let mut total_bytes_sent = 0;
        for _ in 0..1000 {
            let stream = &self.stream;
            match stream.deref().write(&data[total_bytes_sent..]) {
                Ok(bytes_sent) => {
                    total_bytes_sent += bytes_sent;
                    if total_bytes_sent == data.len() {
                        return SendStatus::Sent;
                    }
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => continue,

                // Others errors are considered fatal for the connection.
                // a Event::Disconnection will be generated later.
                Err(err) => {
                    log::error!("TCP receive error: {}", err);
                    return SendStatus::ResourceNotFound; // should not happen
                }
            }
        }

        // abort after retry 1000 times
        return SendStatus::SendAbort;
    }

    #[inline(always)]
    fn pending(&self) -> PendingStatus {
        let status = check_tcp_stream_ready(&self.stream);

        if status == PendingStatus::Ready {
            if let Some(keepalive) = &self.keepalive_opt {
                //
                let socket = tcp_stream_to_socket(&self.stream);

                if let Err(e) = socket.set_tcp_keepalive(keepalive) {
                    log::warn!("TCP set keepalive error: {}", e);
                }

                // Don't drop so the underlying socket is not closed.
                forget(socket);
            }
        }

        status
    }
}

///
pub(crate) struct LocalResource {
    pub listener: TcpListener,
    pub keepalive_opt: Option<TcpKeepalive>,
}

impl Resource for LocalResource {
    #[inline(always)]
    fn source(&mut self) -> &mut dyn Source {
        &mut self.listener
    }
}

impl Local for LocalResource {
    type Remote = RemoteResource;

    fn listen_with(config: ListenConfig, addr: SocketAddr) -> io::Result<ListeningInfo<Self>> {
        //
        let socket = Socket::new(
            match addr {
                SocketAddr::V4 { .. } => Domain::IPV4,
                SocketAddr::V6 { .. } => Domain::IPV6,
            },
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        socket.set_nonblocking(true)?;
        socket.set_nodelay(true)?;
        socket.set_reuse_address(true)?;

        #[cfg(unix)]
        if let Some(bind_device) = config.bind_device_opt {
            let device = CString::new(bind_device)?;

            #[cfg(not(target_os = "macos"))]
            socket.bind_device(Some(device.as_bytes()))?;

            #[cfg(target_os = "macos")]
            match NonZeroU32::new(unsafe { libc::if_nametoindex(device.as_ptr()) }) {
                Some(index) => socket.bind_device_by_index(Some(index))?,
                None => {
                    return Err(io::Error::new(
                        ErrorKind::NotFound,
                        "Bind device interface not found",
                    ))
                }
            }
        }

        socket.bind(&addr.into())?;
        socket.listen(LISTENER_BACKLOG)?;

        let listener = TcpListener::from_std(socket.into());

        let local_addr = listener.local_addr().unwrap();
        Ok(ListeningInfo {
            local: {
                LocalResource {
                    //
                    listener,
                    keepalive_opt: config.keepalive_opt,
                }
            },
            local_addr,
        })
    }

    #[inline(always)]
    fn accept(&self, mut accept_remote: impl FnMut(AcceptedType<'_>)) {
        loop {
            match self.listener.accept() {
                Ok((stream, addr)) => {
                    accept_remote(AcceptedType::Remote(
                        addr,
                        stream,
                        Box::new(TcpAcceptPayload {
                            //
                            keepalive_opt: self.keepalive_opt.clone(),
                        }),
                    ));
                }
                Err(ref err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => break log::error!("TCP accept error: {}", err), // Should not happen
            }
        }
    }
}

/// Check if a TcpStream can be considered connected.
#[inline(always)]
pub fn check_tcp_stream_ready(stream: &TcpStream) -> PendingStatus {
    // A multiplatform non-blocking way to determine if the TCP stream is connected:
    // Extracted from: https://github.com/tokio-rs/mio/issues/1486
    if let Ok(Some(_)) = stream.take_error() {
        return PendingStatus::Disconnected;
    }
    match stream.peer_addr() {
        Ok(_) => PendingStatus::Ready,
        Err(err) if err.kind() == io::ErrorKind::NotConnected => PendingStatus::Incomplete,
        Err(err) if err.kind() == io::ErrorKind::InvalidInput => PendingStatus::Incomplete,
        Err(_) => PendingStatus::Disconnected,
    }
}

/// Get socket from tcp stream
#[inline(always)]
pub fn tcp_stream_to_socket(stream: &TcpStream) -> Socket {
    #[cfg(target_os = "windows")]
    unsafe {
        Socket::from_raw_socket(stream.as_raw_socket())
    }
    #[cfg(not(target_os = "windows"))]
    unsafe {
        Socket::from_raw_fd(stream.as_raw_fd())
    }
}
