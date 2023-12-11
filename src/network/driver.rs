use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use mio::net::TcpStream;
use socket2::TcpKeepalive;

use crate::node::NodeHandler;
use crate::util::unsafe_any::UnsafeAny;

use super::adapter::AcceptedType;
use super::endpoint::Endpoint;
use super::net_event::NetEvent;
use super::resource_id::ResourceId;
use super::RemoteAddr;

#[derive(Clone, Debug, Default)]
pub struct ConnectConfig {
    pub bind_device_opt: Option<String>,
    pub source_address_opt: Option<SocketAddr>,
    pub keepalive_opt: Option<TcpKeepalive>,
}

impl ConnectConfig {
    /// Bind the TCP connection to a specific interface, identified by its name. This option works
    /// in Unix, on other systems, it will be ignored.
    pub fn with_bind_device(mut self, device: String) -> Self {
        self.bind_device_opt = Some(device);
        self
    }

    /// Enables TCP keepalive settings on the socket.
    pub fn with_keepalive(mut self, keepalive: TcpKeepalive) -> Self {
        self.keepalive_opt = Some(keepalive);
        self
    }

    /// Specify the source address and port.
    pub fn with_source_address(mut self, source_address: SocketAddr) -> Self {
        self.source_address_opt = Some(source_address);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct ListenConfig {
    pub bind_device_opt: Option<String>,
    pub keepalive_opt: Option<TcpKeepalive>,

    pub cert_path: PathBuf,
    pub pri_key_path: PathBuf,
}

impl ListenConfig {
    /// Bind the TCP listener to a specific interface, identified by its name. This option works in
    /// Unix, on other systems, it will be ignored.
    pub fn with_bind_device(mut self, device: String) -> Self {
        self.bind_device_opt = Some(device);
        self
    }

    /// Enables TCP keepalive settings on client connection sockets.
    pub fn with_keepalive(mut self, keepalive: TcpKeepalive) -> Self {
        self.keepalive_opt = Some(keepalive);
        self
    }
}

pub trait EventProcessor {
    fn process_read(&mut self, id: ResourceId, callback: &mut dyn FnMut(NetEvent));
    fn process_write(&mut self, id: ResourceId, callback: &mut dyn FnMut(NetEvent));
    fn process_accept_register_remote(
        &mut self,
        local_pair: (ResourceId, SocketAddr),
        remote_pair: (ResourceId, SocketAddr),
        stream: TcpStream,
        payload: Box<dyn UnsafeAny + Send>,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    );
    fn process_connect_register_remote(
        &mut self,
        local_addr: SocketAddr,
        remote_pair: (ResourceId, SocketAddr),
        stream: TcpStream,
        keepalive_opt: Option<TcpKeepalive>,
        domain_opt: Option<String>,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    );
    fn process_listen(
        &mut self,
        config: ListenConfig,
        local_id: ResourceId,
        addr: SocketAddr,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send>,
    );
    fn process_connect(
        &mut self,
        config: ConnectConfig,
        remote_id: ResourceId,
        raddr: RemoteAddr,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    );
    fn process_send(&mut self, endpoint: Endpoint, data: &[u8]);
    fn process_close(
        &mut self,
        id: ResourceId,
        callback: Box<dyn FnOnce(&NodeHandler, bool) + Send>,
    );
    fn process_is_ready(
        &mut self,
        id: ResourceId,
        callback: Box<dyn FnOnce(&NodeHandler, Option<bool>) + Send>,
    );
}

///
pub struct RemoteProperties {
    pub peer_addr: SocketAddr,
    pub local: Option<ResourceId>,
    pub ready: AtomicBool,
}

impl RemoteProperties {
    ///
    pub fn new(peer_addr: SocketAddr, local: Option<ResourceId>) -> Self {
        Self { peer_addr, local, ready: AtomicBool::new(false) }
    }

    ///
    #[inline(always)]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    ///
    #[inline(always)]
    pub fn mark_as_ready(&self) {
        self.ready.store(true, Ordering::Relaxed);
    }
}

///
pub struct LocalProperties;

impl std::fmt::Display for AcceptedType<'_> {
    #[inline(always)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            AcceptedType::Remote(addr, _, _) => format!("Remote({addr})"),
            AcceptedType::Data(addr, _) => format!("Data({addr})"),
        };
        write!(f, "AcceptedType::{string}")
    }
}
