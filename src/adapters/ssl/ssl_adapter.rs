use socket2::TcpKeepalive;
#[cfg(unix)]
use std::ffi::CString;
use std::net::SocketAddr;
use std::os::raw::c_int;
use std::path::PathBuf;

/// The maximum length of the pending (unaccepted) connection queue of a listener.
pub const LISTENER_BACKLOG: c_int = 1024;

#[derive(Clone, Debug, Default)]
pub struct SslConnectConfig {
    bind_device: Option<String>,
    source_address: Option<SocketAddr>,
    keepalive: Option<TcpKeepalive>,
}

impl SslConnectConfig {
    /// Bind the TCP connection to a specific interface, identified by its name. This option works
    /// in Unix, on other systems, it will be ignored.
    pub fn with_bind_device(mut self, device: String) -> Self {
        self.bind_device = Some(device);
        self
    }

    /// Enables TCP keepalive settings on the socket.
    pub fn with_keepalive(mut self, keepalive: TcpKeepalive) -> Self {
        self.keepalive = Some(keepalive);
        self
    }

    /// Specify the source address and port.
    pub fn with_source_address(mut self, source_address: SocketAddr) -> Self {
        self.source_address = Some(source_address);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct SslListenConfig {
    bind_device: Option<String>,
    keepalive: Option<TcpKeepalive>,

    cert_path: PathBuf,
    pri_key_path: PathBuf,
}

impl SslListenConfig {
    ///
    pub fn new(
        bind_device: Option<String>,
        keepalive: Option<TcpKeepalive>,
        cert_path: PathBuf,
        pri_key_path: PathBuf,
    ) -> Self {
        Self { bind_device, keepalive, cert_path, pri_key_path }
    }

    /// Bind the TCP listener to a specific interface, identified by its name. This option works in
    /// Unix, on other systems, it will be ignored.
    pub fn with_bind_device(mut self, device: String) -> Self {
        self.bind_device = Some(device);
        self
    }

    /// Enables TCP keepalive settings on client connection sockets.
    pub fn with_keepalive(mut self, keepalive: TcpKeepalive) -> Self {
        self.keepalive = Some(keepalive);
        self
    }
}

///
#[cfg(feature = "native-tls")]
pub mod encryption {
    pub(crate) struct SslAdapter;
    impl Adapter for SslAdapter {
        type Remote = RemoteResource;
        type Local = LocalResource;
    }

    pub(crate) struct RemoteResource {
        stream: Mutex<SslStream<TcpStream>>,
        keepalive: Option<TcpKeepalive>,
    }

    impl Resource for RemoteResource {
        fn source(&mut self) -> &mut dyn Source {
            let stream = self.stream.get_mut();
            match stream {
                SslStream::NativeTls(ref mut s) => {
                    //
                    s.get_mut()
                }
            }
        }
    }

    impl Remote for RemoteResource {
        fn connect_with(
            config: TransportConnect,
            remote_addr: RemoteAddr,
        ) -> IoResult<ConnectionInfo<Self>> {
            let config = match config {
                TransportConnect::Ssl(config) => config,
                _ => panic!("Internal error: Got wrong config"),
            };

            assert!(remote_addr.is_string());
            let uri: Uri = remote_addr.to_string().parse().unwrap();
            let peer_addr = *remote_addr.socket_addr();

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

            if let Some(source_address) = config.source_address {
                socket.bind(&source_address.into())?;
            }

            #[cfg(unix)]
            if let Some(bind_device) = config.bind_device {
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

            match socket.connect(&peer_addr.into()) {
                #[cfg(unix)]
                Err(e) if e.raw_os_error() != Some(libc::EINPROGRESS) => return Err(e),
                #[cfg(windows)]
                Err(e) if e.kind() != io::ErrorKind::WouldBlock => return Err(e),
                _ => {}
            }

            let raw = TcpStream::from_std(socket.into());
            let local_addr = raw.local_addr()?;

            let domain = match uri.host() {
                Some(d) => Ok(d.to_string()),
                None => Err(io::Error::new(ErrorKind::NotFound, "Host not found")),
            }?;

            let wrapped_stream =
                ssl_connector::encryption::native_tls::wrap_stream(raw, domain.as_str(), None);
            match wrapped_stream {
                Ok(s) => {
                    //
                    Ok(ConnectionInfo {
                        remote: Self {
                            //
                            stream: Mutex::new(s),
                            keepalive: config.keepalive,
                        },
                        local_addr,
                        peer_addr,
                    })
                }
                Err(_e) => {
                    //
                    std::unreachable!()
                }
            }
        }

        fn receive(&self, mut process_data: impl FnMut(NetPacketGuard)) -> ReadStatus {
            loop {
                //
                let mut stream = self.stream.lock();
                let deref_stream = stream.deref_mut();
                match deref_stream {
                    SslStream::NativeTls(ref mut s) => {
                        //
                        std::unimplemented!()
                    }
                }
            }
        }

        fn send(&self, data: &[u8]) -> SendStatus {
            // TODO: The current implementation implies an active waiting,
            // improve it using POLLIN instead to avoid active waiting.
            // Note: Despite the fear that an active waiting could generate,
            // this only occurs in the case when the receiver is full because reads slower that it sends.
            let mut total_bytes_sent = 0;
            loop {
                //
                let mut stream = self.stream.lock();
                let deref_stream = stream.deref_mut();
                match deref_stream {
                    SslStream::NativeTls(_) => {
                        std::unreachable!()
                    }
                }
            }
        }

        fn pending(&self, _readiness: Readiness) -> PendingStatus {
            let stream = self.stream.lock();
            let status = check_stream_ready(&stream);
            if status == PendingStatus::Ready {
                if let Some(keepalive) = &self.keepalive {
                    //
                    let socket = stream_to_socket(&stream);

                    //
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

    #[inline(always)]
    fn read_ssl_stream<C: Sized, S: Read + Write + Sized>(stream: StreamOwned<S>) -> ReadStatus {
        //
        let mut input_buffer = take_small_packet();
        let buf = input_buffer.extend(SMALL_PACKET_MAX_SIZE);

        //
        match stream.read(buf) {
            Ok(0) => return ReadStatus::Disconnected,
            Ok(size) => {
                //
                input_buffer.truncate(size);
                process_data(input_buffer)
            }
            Err(ref err) if err.kind() == ErrorKind::Interrupted => ReadStatus::WaitNextEvent,
            Err(ref err) if err.kind() == ErrorKind::WouldBlock => ReadStatus::WaitNextEvent,
            Err(ref err) if err.kind() == ErrorKind::ConnectionReset => ReadStatus::Disconnected,
            Err(err) => {
                log::error!("TCP receive error: {}", err);
                ReadStatus::Disconnected // should not happen
            }
        }
    }

    #[inline(always)]
    fn check_stream_ready(stream: &SslStream<TcpStream>) -> PendingStatus {
        match stream {
            SslStream::NativeTls(_) => {
                std::unimplemented!()
            }
        }
    }

    #[inline(always)]
    fn stream_to_socket(stream: &SslStream<TcpStream>) -> Socket {
        match stream {
            SslStream::RustlsStreamAcceptor(_) => {
                std::unreachable!()
            }
            SslStream::RustlsStreamEncrypted(ref s) => {
                //
                let s = s.get_ref();
                crate::adapters::tcp::tcp_stream_to_socket(&s)
            }
        }
    }

    pub(crate) struct LocalResource {
        listener: TcpListener,
        keepalive: Option<TcpKeepalive>,
    }

    impl Resource for LocalResource {
        fn source(&mut self) -> &mut dyn Source {
            &mut self.listener
        }
    }

    impl Local for LocalResource {
        type Remote = RemoteResource;

        fn listen_with(config: TransportListen, addr: SocketAddr) -> IoResult<ListeningInfo<Self>> {
            let config = match config {
                TransportListen::Ssl(config) => config,
                _ => panic!("Internal error: Got wrong config"),
            };

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
            if let Some(bind_device) = config.bind_device {
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
                local: { LocalResource { listener, keepalive: config.keepalive } },
                local_addr,
            })
        }

        fn accept(&self, mut accept_remote: impl FnMut(AcceptedType<'_, Self::Remote>)) {
            loop {
                match self.listener.accept() {
                    Ok((s, addr)) => {
                        //
                        log::info!("accept along with remote addr: {:?}", addr);

                        let acceptor = ssl_acceptor::encryption::native_tls::create_acceptor();
                        accept_remote(AcceptedType::Remote(
                            addr,
                            RemoteResource {
                                //
                                stream: Mutex::new(SslStream::NativeTlsStreamAcceptor(s, acceptor)),
                                keepalive: self.keepalive.clone(),
                            },
                        ))
                    }
                    Err(ref err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(err) => break log::error!("TCP accept error: {}", err), // Should not happen
                }
            }
        }
    }
}

///
#[cfg(feature = "__rustls-tls")]
pub mod encryption {
    use std::io::{self, ErrorKind, Read, Result as IoResult, Write};
    use std::mem::forget;
    use std::net::SocketAddr;
    #[cfg(target_os = "macos")]
    use std::num::NonZeroU32;
    use std::ops::DerefMut;
    use std::sync::Arc;

    use http::Uri;
    use mio::event::Source;
    use mio::net::{TcpListener, TcpStream};
    use parking_lot::Mutex;
    use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};

    use net_packet::{take_small_packet, NetPacketGuard, SMALL_PACKET_MAX_SIZE};

    use crate::adapters::ssl::{ssl_acceptor, ssl_connector, ssl_stream::SslStream};
    use crate::network::adapter::{
        AcceptedType, Adapter, ConnectionInfo, ListeningInfo, Local, PendingStatus, ReadStatus,
        Remote, Resource, SendStatus,
    };
    use crate::network::{RemoteAddr, TransportConnect, TransportListen};

    pub(crate) struct SslAdapter;

    impl Adapter for SslAdapter {
        type Remote = RemoteResource;
        type Local = LocalResource;
    }

    pub(crate) struct RemoteResource {
        stream: Mutex<SslStream<TcpStream>>,
        keepalive: Option<TcpKeepalive>,
    }

    impl Resource for RemoteResource {
        fn source(&mut self) -> &mut dyn Source {
            let stream = self.stream.get_mut();
            match stream {
                SslStream::RustlsStreamAcceptor(ref mut opt) => {
                    //
                    match opt {
                        Some(s) => &mut s.0,
                        None => {
                            //
                            std::panic!("IT SHOULD NEVER HAPPEN!!!")
                        }
                    }
                }
                SslStream::RustlsClientConnection(ref mut s) => {
                    //
                    &mut s.sock
                }
                SslStream::RustlsServerConnection(ref mut s) => {
                    //
                    &mut s.sock
                }
            }
        }
    }

    impl Remote for RemoteResource {
        fn connect_with(
            config: TransportConnect,
            remote_addr: RemoteAddr,
        ) -> IoResult<ConnectionInfo<Self>> {
            let config = match config {
                TransportConnect::Ssl(config) => config,
                _ => panic!("Internal error: Got wrong config"),
            };

            assert!(remote_addr.is_string());
            let uri: Uri = remote_addr.to_string().parse().unwrap();
            let peer_addr = *remote_addr.socket_addr();

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

            if let Some(source_address) = config.source_address {
                socket.bind(&source_address.into())?;
            }

            #[cfg(unix)]
            if let Some(bind_device) = config.bind_device {
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

            match socket.connect(&peer_addr.into()) {
                #[cfg(unix)]
                Err(e) if e.raw_os_error() != Some(libc::EINPROGRESS) => return Err(e),
                #[cfg(windows)]
                Err(e) if e.kind() != io::ErrorKind::WouldBlock => return Err(e),
                _ => {}
            }

            let raw = TcpStream::from_std(socket.into());
            let local_addr = raw.local_addr()?;

            let domain = match uri.host() {
                Some(d) => Ok(d.to_string()),
                None => Err(io::Error::new(ErrorKind::NotFound, "Host not found")),
            }?;

            let wrapped_stream =
                ssl_connector::encryption::rustls::wrap_stream(raw, domain.as_str(), None);
            match wrapped_stream {
                Ok(s) => {
                    //
                    Ok(ConnectionInfo {
                        remote: Self {
                            //
                            stream: Mutex::new(s),
                            keepalive: config.keepalive,
                        },
                        local_addr,
                        peer_addr,
                    })
                }
                Err(_e) => {
                    //
                    std::unreachable!()
                }
            }
        }

        fn receive(&self, mut process_data: impl FnMut(NetPacketGuard)) -> ReadStatus {
            loop {
                //
                let mut stream = self.stream.lock();
                let deref_stream = stream.deref_mut();
                match deref_stream {
                    SslStream::RustlsStreamAcceptor(ref mut opt) => {
                        //
                        if let Some((raw, acceptor, config)) = opt.take() {
                            //
                            let raw_id = std::format!("{:?}", raw);

                            //
                            let wrapped_stream = ssl_acceptor::encryption::rustls::wrap_stream(
                                raw, acceptor, config,
                            );
                            match wrapped_stream {
                                Ok(s) => {
                                    // Switch stream state to the wrapped stream
                                    *stream = s;
                                }
                                Err((err, raw, acceptor, config)) => {
                                    // Recover stream state
                                    *stream = SslStream::RustlsStreamAcceptor(Some((
                                        raw, acceptor, config,
                                    )));

                                    //
                                    if err.kind() == io::ErrorKind::WouldBlock {
                                        continue;
                                    } else {
                                        //
                                        log::error!(
                                            "RustlsStreamAcceptor wrapped stream error: {}, socket: {}",
                                            err, raw_id
                                        );
                                        break ReadStatus::Disconnected;
                                    }
                                }
                            }
                        }
                    }
                    SslStream::RustlsClientConnection(ref mut s) => {
                        //
                        let mut input_buffer = take_small_packet();
                        let buf = input_buffer.extend(SMALL_PACKET_MAX_SIZE);

                        //
                        match s.read(buf) {
                            Ok(0) => break ReadStatus::Disconnected,
                            Ok(size) => {
                                //
                                input_buffer.truncate(size);
                                process_data(input_buffer);
                                continue;
                            }
                            Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                            Err(ref err) if err.kind() == ErrorKind::WouldBlock => {
                                break ReadStatus::WaitNextEvent
                            }
                            Err(ref err) if err.kind() == ErrorKind::ConnectionReset => {
                                break ReadStatus::Disconnected
                            }
                            Err(err) => {
                                log::error!("RustlsClientConnection read error: {}", err);
                                break ReadStatus::Disconnected; // should not happen
                            }
                        }
                    }
                    SslStream::RustlsServerConnection(ref mut s) => {
                        //
                        let mut input_buffer = take_small_packet();
                        let buf = input_buffer.extend(SMALL_PACKET_MAX_SIZE);

                        //
                        match s.read(buf) {
                            Ok(0) => break ReadStatus::Disconnected,
                            Ok(size) => {
                                //
                                input_buffer.truncate(size);
                                process_data(input_buffer);
                                continue;
                            }
                            Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                            Err(ref err) if err.kind() == ErrorKind::WouldBlock => {
                                break ReadStatus::WaitNextEvent
                            }
                            Err(ref err) if err.kind() == ErrorKind::ConnectionReset => {
                                break ReadStatus::Disconnected
                            }
                            Err(err) => {
                                log::error!(
                                    "RustlsServerConnection read error: {} socket: {:?}",
                                    err,
                                    s.sock
                                );
                                break ReadStatus::Disconnected;
                            }
                        }
                    }
                }
            }
        }

        fn send(&self, data: &[u8]) -> SendStatus {
            // TODO: The current implementation implies an active waiting,
            // improve it using POLLIN instead to avoid active waiting.
            // Note: Despite the fear that an active waiting could generate,
            // this only occurs in the case when the receiver is full because reads slower that it sends.
            let mut total_bytes_sent = 0_usize;
            loop {
                //
                let mut stream = self.stream.lock();
                let deref_stream = stream.deref_mut();
                match deref_stream {
                    SslStream::RustlsStreamAcceptor(_) => {
                        std::unreachable!()
                    }
                    SslStream::RustlsClientConnection(ref mut s) => {
                        //
                        match s.write(&data[total_bytes_sent..]) {
                            Ok(bytes_sent) => {
                                total_bytes_sent += bytes_sent;
                                if total_bytes_sent == data.len() {
                                    //
                                    break SendStatus::Sent;
                                }
                            }
                            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                                //
                                continue;
                            }

                            // Others errors are considered fatal for the connection.
                            // a Event::Disconnection will be generated later.
                            Err(err) => {
                                log::error!("Ssl receive error: {}", err);
                                break SendStatus::ResourceNotFound; // should not happen
                            }
                        }
                    }
                    SslStream::RustlsServerConnection(ref mut s) => {
                        //
                        match s.write(&data[total_bytes_sent..]) {
                            Ok(bytes_sent) => {
                                total_bytes_sent += bytes_sent;
                                if total_bytes_sent == data.len() {
                                    //
                                    break SendStatus::Sent;
                                }
                            }
                            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                                //
                                continue;
                            }

                            // Others errors are considered fatal for the connection.
                            // a Event::Disconnection will be generated later.
                            Err(err) => {
                                log::error!("Ssl receive error: {}", err);
                                break SendStatus::ResourceNotFound; // should not happen
                            }
                        }
                    }
                }
            }
        }

        fn pending(&self) -> PendingStatus {
            let stream = self.stream.lock();
            let status = check_stream_ready(&stream);
            if status == PendingStatus::Ready {
                if let Some(keepalive) = &self.keepalive {
                    //
                    let socket = stream_to_socket(&stream);

                    //
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

    #[inline(always)]
    fn check_stream_ready(stream: &SslStream<TcpStream>) -> PendingStatus {
        match stream {
            SslStream::RustlsStreamAcceptor(ref opt) => {
                crate::adapters::tcp::check_tcp_stream_ready(&opt.as_ref().unwrap().0)
            }
            SslStream::RustlsClientConnection(ref s) => {
                crate::adapters::tcp::check_tcp_stream_ready(&s.sock)
            }
            SslStream::RustlsServerConnection(ref s) => {
                crate::adapters::tcp::check_tcp_stream_ready(&s.sock)
            }
        }
    }

    #[inline(always)]
    fn stream_to_socket(stream: &SslStream<TcpStream>) -> Socket {
        match stream {
            SslStream::RustlsStreamAcceptor(_) => {
                std::unreachable!()
            }
            SslStream::RustlsClientConnection(ref s) => {
                crate::adapters::tcp::tcp_stream_to_socket(&s.sock)
            }
            SslStream::RustlsServerConnection(ref s) => {
                crate::adapters::tcp::tcp_stream_to_socket(&s.sock)
            }
        }
    }

    pub(crate) struct LocalResource {
        listener: TcpListener,
        keepalive: Option<TcpKeepalive>,

        server_config: Arc<rustls::ServerConfig>,
    }

    impl Resource for LocalResource {
        fn source(&mut self) -> &mut dyn Source {
            &mut self.listener
        }
    }

    impl Local for LocalResource {
        type Remote = RemoteResource;

        fn listen_with(config: TransportListen, addr: SocketAddr) -> IoResult<ListeningInfo<Self>> {
            let config = match config {
                TransportListen::Ssl(config) => config,
                _ => panic!("Internal error: Got wrong config"),
            };

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
            if let Some(bind_device) = config.bind_device {
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
            socket.listen(super::LISTENER_BACKLOG)?;

            let listener = TcpListener::from_std(socket.into());

            let server_config = ssl_acceptor::encryption::rustls::create_server_config(
                &config.cert_path,
                &config.pri_key_path,
            );

            let local_addr = listener.local_addr().unwrap();

            Ok(ListeningInfo {
                local: { LocalResource { listener, keepalive: config.keepalive, server_config } },
                local_addr,
            })
        }

        fn accept(&self, mut accept_remote: impl FnMut(AcceptedType<'_, Self::Remote>)) {
            loop {
                match self.listener.accept() {
                    Ok((s, addr)) => {
                        //
                        //log::info!("accept along with remote addr: {:?}", addr);

                        let acceptor = ssl_acceptor::encryption::rustls::create_acceptor();
                        accept_remote(AcceptedType::Remote(
                            addr,
                            RemoteResource {
                                //
                                stream: Mutex::new(SslStream::RustlsStreamAcceptor(Some((
                                    s,
                                    acceptor,
                                    self.server_config.clone(),
                                )))),
                                keepalive: self.keepalive.clone(),
                            },
                        ))
                    }
                    Err(ref err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(err) => break log::error!("TCP accept error: {}", err), // Should not happen
                }
            }
        }
    }
}
