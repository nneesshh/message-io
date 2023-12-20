#[cfg(unix)]
use std::ffi::CString;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
#[cfg(target_os = "macos")]
use std::num::NonZeroU32;
use std::os::raw::c_int;
use std::sync::Arc;

use mio::event::Source;
use mio::net::{TcpListener, TcpStream};
use net_packet::{take_small_packet, NetPacketGuard};
use parking_lot::Mutex;
pub use socket2::TcpKeepalive;

use socket2::{Domain, Protocol, Socket, Type};
use url::Url;

use crate::adapters::tcp::{check_tcp_stream_ready};
use crate::network::adapter::{
    AcceptedType, Adapter, ListeningInfo, Local, PendingStatus, ReadStatus, Remote, Resource,
    SendStatus,
};
use crate::network::driver::ListenConfig;

use super::client::ws_connect;
use super::handshake::client::ClientHandshake;
use super::handshake::server::{NoCallback, ServerHandshake};
use super::handshake::{MidHandshake, HandshakeError};
use super::protocol::{WebSocket, Message};
use super::error::Error;
use super::server::ws_accept;

/// The maximum length of the pending (unaccepted) connection queue of a listener.
pub const LISTENER_BACKLOG: c_int = 1024;

/// Max message size for default config
// From https://docs.rs/tungstenite/0.13.0/src/tungstenite/protocol/mod.rs.html#65
pub const MAX_PAYLOAD_LEN: usize = 32 << 20;

/// Plain struct used as a returned value of [`Remote::connect_with()`]
pub struct WsConnectionInfo {
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
pub struct WsAcceptPayload {
    pub keepalive_opt: Option<TcpKeepalive>,
}

///
pub struct WsConnectPayload {
    pub url: Url,
    pub keepalive_opt: Option<TcpKeepalive>,
}

///
pub(crate) struct WsAdapter;

impl Adapter for WsAdapter {
    type Remote = RemoteResource;
    type Local = LocalResource;
}

/// This struct is used to avoid the tungstenite handshake to take the ownership of the stream
/// an drop it without allow to the driver to deregister from the poll.
/// It can be removed when this issue is resolved:
/// https://github.com/snapview/tungstenite-rs/issues/51
pub struct ArcTcpStream(Arc<TcpStream>);

impl From<TcpStream> for ArcTcpStream {
    fn from(stream: TcpStream) -> Self {
        Self(Arc::new(stream))
    }
}

impl io::Read for ArcTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl io::Write for ArcTcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}

impl Clone for ArcTcpStream {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

///
pub enum PendingHandshake {
    Connect(Url, ArcTcpStream),
    Accept(ArcTcpStream),
    Client(MidHandshake<ClientHandshake<ArcTcpStream>>),
    Server(MidHandshake<ServerHandshake<ArcTcpStream, NoCallback>>),
}

///
#[allow(clippy::large_enum_variant)]
pub enum RemoteState {
    WebSocket(WebSocket<ArcTcpStream>),
    Handshake(Option<PendingHandshake>),
    Error(ArcTcpStream),
}

///
pub(crate) struct RemoteResource {
    pub state: Mutex<RemoteState>,
    pub keepalive_opt: Option<TcpKeepalive>,
}

impl RemoteResource {
    fn io_error_to_read_status(err: &io::Error) -> ReadStatus {
        if err.kind() == io::ErrorKind::WouldBlock {
            ReadStatus::WaitNextEvent
        } else if err.kind() == io::ErrorKind::ConnectionReset {
            ReadStatus::Disconnected
        } else {
            log::error!("WS receive error: {}", err);
            ReadStatus::Disconnected // should not happen
        }
    }
}

impl Resource for RemoteResource {
    #[inline(always)]
    fn source(&mut self) -> &mut dyn Source {
        match self.state.get_mut() {
            RemoteState::WebSocket(web_socket) => {
                Arc::get_mut(&mut web_socket.get_mut().0).unwrap()
            }
            RemoteState::Handshake(Some(handshake)) => match handshake {
                PendingHandshake::Connect(_, stream) => Arc::get_mut(&mut stream.0).unwrap(),
                PendingHandshake::Accept(stream) => Arc::get_mut(&mut stream.0).unwrap(),
                PendingHandshake::Client(handshake) => {
                    Arc::get_mut(&mut handshake.get_mut().get_mut().0).unwrap()
                }
                PendingHandshake::Server(handshake) => {
                    Arc::get_mut(&mut handshake.get_mut().get_mut().0).unwrap()
                }
            },
            RemoteState::Handshake(None) => unreachable!(),
            RemoteState::Error(stream) => Arc::get_mut(&mut stream.0).unwrap(),
        }
    }
}

impl Remote for RemoteResource {
    //#[inline(always)]
    fn receive(&self, process_data: &mut dyn FnMut(NetPacketGuard)) -> ReadStatus {
        loop {
            let mut input_buffer = take_small_packet();

            // "emulates" full duplex for the websocket case locking here and not outside the loop.
            let mut state = self.state.lock();
            let deref_state = &mut *state;
            match deref_state {
                RemoteState::WebSocket(web_socket) => match web_socket.read() {
                    Ok(message) => match message {
                        Message::Binary(data) => {
                            // As an optimization.
                            // Fast check to know if there is more data to avoid call
                            // WebSocket::read_message() again.
                            // TODO: investigate why this code doesn't work in windows.
                            // Seems like windows consume the `WouldBlock` notification
                            // at peek() when it happens, and the poll never wakes it again.
                            #[cfg(not(target_os = "windows"))]
                            let _peek_result = web_socket.get_ref().0.peek(&mut [0; 0]);

                            // We can not call process_data while the socket is blocked.
                            // The user could lock it again if sends from the callback.
                            drop(state);

                            //
                            input_buffer.append_slice(data.as_slice());
                            process_data(input_buffer);

                            #[cfg(not(target_os = "windows"))]
                            if let Err(err) = _peek_result {
                                break Self::io_error_to_read_status(&err);
                            }
                        }
                        Message::Close(_) => break ReadStatus::Disconnected,
                        _ => continue,
                    },
                    Err(Error::Io(ref err)) => break Self::io_error_to_read_status(err),
                    Err(err) => {
                        log::error!("WS receive error: {}", err);
                        break ReadStatus::Disconnected; // should not happen
                    }
                },
                RemoteState::Handshake(_) => unreachable!(),
                RemoteState::Error(_) => unreachable!(),
            }
        }
    }

    //#[inline(always)]
    fn send(&self, data: &[u8]) -> SendStatus {
        let mut state = self.state.lock();
        let deref_state = &mut *state;
        match deref_state {
            RemoteState::WebSocket(web_socket) => {
                let message = Message::Binary(data.to_vec());
                let mut result = web_socket.send(message);
                loop {
                    match result {
                        Ok(_) => break SendStatus::Sent,
                        Err(Error::Io(ref err)) if err.kind() == ErrorKind::WouldBlock => {
                            result = web_socket.flush();
                        }
                        Err(Error::Capacity(_)) => break SendStatus::MaxPacketSizeExceeded,
                        Err(err) => {
                            log::error!("WS send error: {}", err);
                            break SendStatus::ResourceNotFound; // should not happen
                        }
                    }
                }
            }
            RemoteState::Handshake(_) => unreachable!(),
            RemoteState::Error(_) => unreachable!(),
        }
    }

    //#[inline(always)]
    fn pending(&self) -> PendingStatus {
        let mut state = self.state.lock();
        let deref_state =&mut *state;
        match deref_state {
            RemoteState::WebSocket(_) => PendingStatus::Ready,
            RemoteState::Handshake(pending) => match pending.take().unwrap() {
                PendingHandshake::Connect(url, stream) => {
                    let tcp_status = check_tcp_stream_ready(&stream.0);
                    if tcp_status != PendingStatus::Ready {
                        // TCP handshake not ready yet.
                        *pending = Some(PendingHandshake::Connect(url, stream));
                        return tcp_status;
                    }
                    let stream_backup = stream.clone();
                    match ws_connect(url, stream) {
                        Ok((web_socket, _)) => {
                            *state = RemoteState::WebSocket(web_socket);
                            PendingStatus::Ready
                        }
                        Err(HandshakeError::Interrupted(mid_handshake)) => {
                            *pending = Some(PendingHandshake::Client(mid_handshake));
                            PendingStatus::Incomplete
                        }
                        Err(HandshakeError::Failure(Error::Io(_))) => {
                            *state = RemoteState::Error(stream_backup);
                            PendingStatus::Disconnected
                        }
                        Err(HandshakeError::Failure(err)) => {
                            *state = RemoteState::Error(stream_backup);
                            log::error!("WS connect handshake error: {}", err);
                            PendingStatus::Disconnected // should not happen
                        }
                    }
                }
                PendingHandshake::Accept(stream) => {
                    let stream_backup = stream.clone();
                    match ws_accept(stream) {
                        Ok(web_socket) => {
                            *state = RemoteState::WebSocket(web_socket);
                            PendingStatus::Ready
                        }
                        Err(HandshakeError::Interrupted(mid_handshake)) => {
                            *pending = Some(PendingHandshake::Server(mid_handshake));
                            PendingStatus::Incomplete
                        }
                        Err(HandshakeError::Failure(Error::Io(_))) => {
                            *state = RemoteState::Error(stream_backup);
                            PendingStatus::Disconnected
                        }
                        Err(HandshakeError::Failure(err)) => {
                            *state = RemoteState::Error(stream_backup);
                            log::error!("WS accept handshake error: {}", err);
                            PendingStatus::Disconnected
                        }
                    }
                }
                PendingHandshake::Client(mid_handshake) => {
                    let stream_backup = mid_handshake.get_ref().get_ref().clone();
                    match mid_handshake.handshake() {
                        Ok((web_socket, _)) => {
                            *state = RemoteState::WebSocket(web_socket);
                            PendingStatus::Ready
                        }
                        Err(HandshakeError::Interrupted(mid_handshake)) => {
                            *pending = Some(PendingHandshake::Client(mid_handshake));
                            PendingStatus::Incomplete
                        }
                        Err(HandshakeError::Failure(Error::Io(_))) => {
                            *state = RemoteState::Error(stream_backup);
                            PendingStatus::Disconnected
                        }
                        Err(HandshakeError::Failure(err)) => {
                            *state = RemoteState::Error(stream_backup);
                            log::error!("WS client handshake error: {}", err);
                            PendingStatus::Disconnected // should not happen
                        }
                    }
                }
                PendingHandshake::Server(mid_handshake) => {
                    let stream_backup = mid_handshake.get_ref().get_ref().clone();
                    match mid_handshake.handshake() {
                        Ok(web_socket) => {
                            *state = RemoteState::WebSocket(web_socket);
                            PendingStatus::Ready
                        }
                        Err(HandshakeError::Interrupted(mid_handshake)) => {
                            *pending = Some(PendingHandshake::Server(mid_handshake));
                            PendingStatus::Incomplete
                        }
                        Err(HandshakeError::Failure(Error::Io(_))) => {
                            *state = RemoteState::Error(stream_backup);
                            PendingStatus::Disconnected
                        }
                        Err(HandshakeError::Failure(err)) => {
                            *state = RemoteState::Error(stream_backup);
                            log::error!("WS server handshake error: {}", err);
                            PendingStatus::Disconnected // should not happen
                        }
                    }
                }
            },
            RemoteState::Error(_) => unreachable!(),
        }
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

    //#[inline(always)]
    fn accept(&self, mut accept_remote: impl FnMut(AcceptedType<'_>)) {
        loop {
            match self.listener.accept() {
                Ok((stream, addr)) => {
                    accept_remote(AcceptedType::Remote(
                        addr,
                        stream,
                        Box::new(WsAcceptPayload {
                            //
                            keepalive_opt: self.keepalive_opt.clone(),
                        }),
                    ));
                }
                Err(ref err) if err.kind() == ErrorKind::WouldBlock => break,
                Err(ref err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => break log::error!("WS accept error: {}", err), // Should not happen
            }
        }
    }
}
