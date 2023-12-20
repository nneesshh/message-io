//! Convenience wrapper for streams to switch between plain TCP and TLS at runtime.
//!
//!  There is no dependency on actual TLS implementations. Everything like
//! `native_tls` or `openssl` will work as long as there is a TLS stream supporting standard
//! `Read + Write` traits.

use std::io::{self, Read, Write};
use std::ops::Deref;
use std::sync::Arc;

use rustls::server::Acceptor;
use rustls::{ClientConnection, ServerConfig, ServerConnection, StreamOwned};

/// Trait to switch TCP_NODELAY.
pub trait NoDelay {
    /// Set the TCP_NODELAY option to the given value.
    fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()>;
}

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
pub enum SslStream<S: Read + Write + Sized> {
    /// Raw stream with acceptor
    RustlsStreamAcceptor(Option<S>, Option<Acceptor>, Arc<ServerConfig>),

    /// Encrypted ClientConnection
    RustlsClientConnection(rustls::StreamOwned<ClientConnection, S>),

    /// Encrypted ServerConnection
    RustlsServerConnection(rustls::StreamOwned<ServerConnection, S>),
}
