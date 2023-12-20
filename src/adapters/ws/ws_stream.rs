use std::net::TcpStream;
use std::{
    fmt::{self, Debug},
    io::{Read, Result as IoResult, Write},
};

/// Stream mode, either plain TCP or TLS.
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    /// Plain mode (`ws://` URL).
    Plain,
    /// TLS mode (`wss://` URL).
    Tls,
}

/// Trait to switch TCP_NODELAY.
pub trait NoDelay {
    /// Set the TCP_NODELAY option to the given value.
    fn set_nodelay(&mut self, nodelay: bool) -> IoResult<()>;
}

impl NoDelay for TcpStream {
    fn set_nodelay(&mut self, nodelay: bool) -> IoResult<()> {
        TcpStream::set_nodelay(self, nodelay)
    }
}

#[cfg(feature = "native-tls")]
impl<S: Read + Write + NoDelay> NoDelay for TlsStream<S> {
    fn set_nodelay(&mut self, nodelay: bool) -> IoResult<()> {
        self.get_mut().set_nodelay(nodelay)
    }
}

#[cfg(feature = "__rustls-tls")]
impl<S, SD, T> NoDelay for StreamOwned<S, T>
where
    S: Deref<Target = rustls::ConnectionCommon<SD>>,
    SD: rustls::SideData,
    T: Read + Write + NoDelay,
{
    fn set_nodelay(&mut self, nodelay: bool) -> IoResult<()> {
        self.sock.set_nodelay(nodelay)
    }
}

/// A stream that might be protected with TLS.
#[non_exhaustive]
pub enum WsStream<S: Read + Write> {
    /// Unencrypted socket stream.
    Plain(S),
}

impl<S: Read + Write + Debug> Debug for WsStream<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(s) => f.debug_tuple("WsStream::Plain").field(s).finish(),
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => f.debug_tuple("WsStream::NativeTls").field(s).finish(),
            #[cfg(feature = "__rustls-tls")]
            Self::Rustls(s) => {
                struct RustlsStreamDebug<'a, S: Read + Write>(
                    &'a rustls::StreamOwned<rustls::ClientConnection, S>,
                );

                impl<'a, S: Read + Write + Debug> Debug for RustlsStreamDebug<'a, S> {
                    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.debug_struct("StreamOwned")
                            .field("conn", &self.0.conn)
                            .field("sock", &self.0.sock)
                            .finish()
                    }
                }

                f.debug_tuple("WsStream::Rustls").field(&RustlsStreamDebug(s)).finish()
            }
        }
    }
}

impl<S: Read + Write> Read for WsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match *self {
            WsStream::Plain(ref mut s) => s.read(buf),
            #[cfg(feature = "native-tls")]
            WsStream::NativeTls(ref mut s) => s.read(buf),
            #[cfg(feature = "__rustls-tls")]
            WsStream::Rustls(ref mut s) => s.read(buf),
        }
    }
}

impl<S: Read + Write> Write for WsStream<S> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match *self {
            WsStream::Plain(ref mut s) => s.write(buf),
            #[cfg(feature = "native-tls")]
            WsStream::NativeTls(ref mut s) => s.write(buf),
            #[cfg(feature = "__rustls-tls")]
            WsStream::Rustls(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> IoResult<()> {
        match *self {
            WsStream::Plain(ref mut s) => s.flush(),
            #[cfg(feature = "native-tls")]
            WsStream::NativeTls(ref mut s) => s.flush(),
            #[cfg(feature = "__rustls-tls")]
            WsStream::Rustls(ref mut s) => s.flush(),
        }
    }
}

impl<S: Read + Write + NoDelay> NoDelay for WsStream<S> {
    fn set_nodelay(&mut self, nodelay: bool) -> IoResult<()> {
        match *self {
            WsStream::Plain(ref mut s) => s.set_nodelay(nodelay),
            #[cfg(feature = "native-tls")]
            WsStream::NativeTls(ref mut s) => s.set_nodelay(nodelay),
            #[cfg(feature = "__rustls-tls")]
            WsStream::Rustls(ref mut s) => s.set_nodelay(nodelay),
        }
    }
}
