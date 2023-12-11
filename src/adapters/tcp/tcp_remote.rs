#[cfg(unix)]
use std::ffi::CString;
use std::io;
use std::net::SocketAddr;

use mio::net::TcpStream;
use socket2::{Domain, Protocol, Socket, Type};

use crate::network::adapter::ConnectionInfo;
use crate::network::driver::ConnectConfig;

///
pub(crate) fn tcp_remote_connect_with(
    config: ConnectConfig,
    peer_addr: SocketAddr,
) -> io::Result<ConnectionInfo> {
    //
    let socket = Socket::new(
        match peer_addr {
            SocketAddr::V4 { .. } => Domain::IPV4,
            SocketAddr::V6 { .. } => Domain::IPV6,
        },
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    socket.set_nonblocking(true)?;
    socket.set_nodelay(true)?;

    if let Some(source_address) = config.source_address_opt {
        socket.bind(&source_address.into())?;
    }

    #[cfg(unix)]
    if let Some(bind_device) = config.bind_device_opt {
        let device = CString::new(bind_device)?;

        #[cfg(not(target_os = "macos"))]
        socket.bind_device(Some(device.as_bytes()))?;

        #[cfg(target_os = "macos")]
        match NonZeroU32::new(unsafe { libc::if_nametoindex(device.as_ptr()) }) {
            Some(index) => socket.bind_device_by_index(Some(index))?,
            None => {
                return Err(io::Error::new(ErrorKind::NotFound, "Bind device interface not found"))
            }
        }
    }

    match socket.connect(&peer_addr.into()) {
        #[cfg(unix)]
        Err(e) if e.raw_os_error() != Some(libc::EINPROGRESS) => return Err(e),
        #[cfg(windows)]
        Err(e) if e.kind() != io::ErrorKind::WouldBlock => return Err(e),
        _ => {}
    }

    let stream = TcpStream::from_std(socket.into());
    let local_addr = stream.local_addr()?;

    //
    Ok(ConnectionInfo {
        stream,
        local_addr,
        peer_addr,
        keepalive_opt: config.keepalive_opt.clone(),
        domain_opt: None,
    })
}
