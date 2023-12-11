//! Convenience wrapper for streams to switch between plain TCP and TLS at runtime.
//!
//!  There is no dependency on actual TLS implementations. Everything like
//! `native_tls` or `openssl` will work as long as there is a TLS stream supporting standard
//! `Read + Write` traits.

use std::io::{self, Read, Write};
use std::sync::Arc;

#[cfg(feature = "native-tls")]
use native_tls_crate::TlsStream;

#[cfg(feature = "__rustls-tls")]
use rustls::server::Acceptor;
use rustls::ServerConfig;
#[cfg(feature = "__rustls-tls")]
use rustls::{ClientConnection, ServerConnection, StreamOwned};
#[cfg(feature = "__rustls-tls")]
use std::ops::Deref;

/// Stream mode, either plain TCP or TLS.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum Mode {
    /// Plain mode (`ws://` URL).
    Plain,
    /// TLS mode (`wss://` URL).
    Tls,
}

/// Trait to switch TCP_NODELAY.
pub trait NoDelay {
    /// Set the TCP_NODELAY option to the given value.
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;
}

#[cfg(feature = "native-tls")]
impl<S: Read + Write + NoDelay> NoDelay for TlsStream<S> {
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
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
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
        self.sock.set_nodelay(nodelay)
    }
}

/// A stream that is protected with TLS.
#[cfg(feature = "native-tls")]
pub enum SslStream<S: Read + Write + Sized> {
    NativeTls(native_tls_crate::TlsStream<S>),
}
#[cfg(feature = "__rustls-tls")]
pub enum SslStream<S: Read + Write + Sized> {
    /// Raw stream with acceptor
    RustlsStreamAcceptor(Option<S>, Option<Acceptor>, Arc<ServerConfig>),

    /// Encrypted ClientConnection
    RustlsClientConnection(rustls::StreamOwned<ClientConnection, S>),

    /// Encrypted ServerConnection
    RustlsServerConnection(rustls::StreamOwned<ServerConnection, S>),
}
