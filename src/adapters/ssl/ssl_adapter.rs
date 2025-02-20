use std::os::raw::c_int;

/// The maximum length of the pending (unaccepted) connection queue of a listener.
pub const LISTENER_BACKLOG: c_int = 1024;

///
pub mod encryption {
    #[cfg(unix)]
    use std::ffi::CString;
    use std::io::{self, ErrorKind, Read, Write};
    use std::mem::forget;
    use std::net::SocketAddr;
    #[cfg(target_os = "macos")]
    use std::num::NonZeroU32;
    use std::ops::DerefMut;
    use std::sync::Arc;

    use mio::event::Source;
    use mio::net::{TcpListener, TcpStream};
    use parking_lot::Mutex;
    use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};

    use rustls::ServerConfig;

    use net_packet::{take_small_packet, NetPacketGuard};

    use crate::adapters::ssl::ssl_acceptor::encryption::rustls::create_server_config;
    use crate::adapters::ssl::{ssl_acceptor, ssl_stream::SslStream};
    use crate::network::adapter::{
        AcceptedType, Adapter, ListeningInfo, Local, PendingStatus, ReadStatus, Remote, Resource,
        SendStatus,
    };
    use crate::network::driver::ListenConfig;

    ///
    pub struct SslAcceptPayload {
        pub keepalive_opt: Option<TcpKeepalive>,
        pub server_config: Arc<ServerConfig>,
    }

    ///
    pub struct SslConnectPayload {
        pub keepalive_opt: Option<TcpKeepalive>,
        pub domain_opt: Option<String>,
    }

    ///
    pub(crate) struct SslAdapter;

    impl Adapter for SslAdapter {
        type Remote = RemoteResource;
        type Local = LocalResource;
    }

    ///
    pub(crate) struct RemoteResource {
        pub stream: Mutex<SslStream<TcpStream>>,
        pub keepalive_opt: Option<TcpKeepalive>,
    }

    impl Resource for RemoteResource {
        fn source(&mut self) -> &mut dyn Source {
            let stream = self.stream.get_mut();
            match stream {
                SslStream::RustlsStreamAcceptor(ref mut raw_opt, _, _) => {
                    //
                    raw_opt.as_mut().unwrap()
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
        fn receive(&self, process_data: &mut dyn FnMut(NetPacketGuard)) -> ReadStatus {
            loop {
                //
                let mut stream = self.stream.lock();
                match stream.deref_mut() {
                    SslStream::RustlsStreamAcceptor(
                        ref mut raw_opt,
                        ref mut acceptor_opt,
                        ref mut server_config,
                    ) => {
                        let raw = raw_opt.take().unwrap();
                        let acceptor = acceptor_opt.take().unwrap();
                        let config = server_config.clone();

                        //
                        let wrapped_stream =
                            ssl_acceptor::encryption::rustls::wrap_stream(raw, acceptor, config);
                        match wrapped_stream {
                            Ok(s) => {
                                // Switch stream state to the wrapped stream
                                *stream = s;
                            }
                            Err((err, raw, acceptor, config)) => {
                                // Recover stream state
                                *stream = SslStream::RustlsStreamAcceptor(
                                    Some(raw),
                                    Some(acceptor),
                                    config,
                                );

                                //
                                if err.kind() == io::ErrorKind::WouldBlock {
                                    continue;
                                } else {
                                    //
                                    log::error!(
                                        "RustlsStreamAcceptor wrapped stream error: {}",
                                        err
                                    );
                                    break ReadStatus::Disconnected;
                                }
                            }
                        }
                    }
                    SslStream::RustlsClientConnection(ref mut s) => {
                        //
                        let mut input_buffer = take_small_packet();
                        let buf = input_buffer.as_write_mut();

                        //
                        match s.read(buf) {
                            Ok(0) => break ReadStatus::Disconnected,
                            Ok(size) => {
                                //
                                input_buffer.end_write(size);
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
                        let buf = input_buffer.as_write_mut();

                        //
                        match s.read(buf) {
                            Ok(0) => break ReadStatus::Disconnected,
                            Ok(size) => {
                                //
                                input_buffer.end_write(size);
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
                    SslStream::RustlsStreamAcceptor(_, _, _) => {
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
                if let Some(keepalive) = &self.keepalive_opt {
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

    ///
    pub(crate) struct LocalResource {
        pub listener: TcpListener,
        pub keepalive_opt: Option<TcpKeepalive>,

        pub server_config: Arc<ServerConfig>,
    }

    impl Resource for LocalResource {
        fn source(&mut self) -> &mut dyn Source {
            &mut self.listener
        }
    }

    impl Local for LocalResource {
        type Remote = RemoteResource;

        //
        fn listen_with(config: ListenConfig, addr: SocketAddr) -> io::Result<ListeningInfo<Self>> {
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
            socket.listen(super::LISTENER_BACKLOG)?;

            let listener = TcpListener::from_std(socket.into());
            let local_addr = listener.local_addr().unwrap();

            let server_config = create_server_config(&config.cert_path, &config.pri_key_path);

            Ok(ListeningInfo {
                local: {
                    LocalResource {
                        //
                        listener,
                        keepalive_opt: config.keepalive_opt,
                        server_config,
                    }
                },
                local_addr,
            })
        }

        fn accept(&self, mut accept_remote: impl FnMut(AcceptedType<'_>)) {
            loop {
                match self.listener.accept() {
                    Ok((s, addr)) => {
                        //
                        accept_remote(AcceptedType::Remote(
                            addr,
                            s,
                            Box::new(SslAcceptPayload {
                                //
                                keepalive_opt: self.keepalive_opt.clone(),
                                server_config: self.server_config.clone(),
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

    #[inline(always)]
    fn check_stream_ready(stream: &SslStream<TcpStream>) -> PendingStatus {
        match stream {
            SslStream::RustlsStreamAcceptor(ref raw_opt, _, _) => {
                let s = raw_opt.as_ref().unwrap();
                crate::adapters::tcp::check_tcp_stream_ready(s)
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
            SslStream::RustlsStreamAcceptor(ref raw_opt, _, _) => {
                let s = raw_opt.as_ref().unwrap();
                crate::adapters::tcp::tcp_stream_to_socket(s)
            }
            SslStream::RustlsClientConnection(ref s) => {
                crate::adapters::tcp::tcp_stream_to_socket(&s.sock)
            }
            SslStream::RustlsServerConnection(ref s) => {
                crate::adapters::tcp::tcp_stream_to_socket(&s.sock)
            }
        }
    }
}
