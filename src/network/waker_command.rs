use std::io;
use std::net::SocketAddr;

use mio::net::TcpStream;
use net_packet::NetPacketGuard;
use socket2::TcpKeepalive;

use crate::{
    network::{Endpoint, RemoteAddr, ResourceId},
    node::NodeHandler,
    util::unsafe_any::UnsafeAny,
};

use super::driver::{ConnectConfig, ListenConfig};

///
pub enum WakerCommand {
    Greet(String),
    AcceptRegisterRemote(
        (ResourceId, SocketAddr),
        (ResourceId, SocketAddr),
        TcpStream,
        Box<dyn UnsafeAny + Send>,
    ),
    ConnectRegisterRemote(
        SocketAddr,
        (ResourceId, SocketAddr),
        TcpStream,
        Option<TcpKeepalive>,
        Option<String>,
        Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ),
    Listen(
        ListenConfig,
        ResourceId,
        SocketAddr,
        Box<dyn FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send>,
    ),
    Connect(
        ConnectConfig,
        ResourceId,
        RemoteAddr,
        Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ),
    Send(Endpoint, NetPacketGuard),
    SendSync(Endpoint, NetPacketGuard, Box<dyn Fn() + Send>),
    Close(ResourceId),
    IsReady(ResourceId, Box<dyn FnOnce(&NodeHandler, Option<bool>) + Send>),
    Stop,
}

impl std::fmt::Debug for WakerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WakerCommand::Greet(greet) => write!(f, "WakerCommand::Greet({greet:?})"),
            WakerCommand::AcceptRegisterRemote(_, remote_pair, _stream, _) => {
                let remote_id: ResourceId = remote_pair.0;
                let remote_addr = remote_pair.1;
                write!(f, "WakerCommand::AcceptRegisterRemote({remote_id:?}-{remote_addr:?})")
            }
            WakerCommand::ConnectRegisterRemote(_, remote_pair, _stream, _, _, _) => {
                let remote_id: ResourceId = remote_pair.0;
                let remote_addr = remote_pair.1;
                write!(f, "WakerCommand::ConnectRegisterRemote({remote_id:?}-{remote_addr:?})")
            }
            WakerCommand::Listen(_, id, addr, _) => {
                write!(f, "WakerCommand::Listen({id:?}-{addr:?})")
            }
            WakerCommand::Connect(_, id, addr, _) => {
                write!(f, "WakerCommand::Connect({id:?}-{addr:?})")
            }
            WakerCommand::Send(endpoint, _) => write!(f, "WakerCommand::Send({endpoint:?})"),
            WakerCommand::SendSync(endpoint, _, _) => {
                write!(f, "WakerCommand::SendSync({endpoint:?})")
            }
            WakerCommand::Close(id) => write!(f, "WakerCommand::Close({id:?}))"),
            WakerCommand::IsReady(id, _) => write!(f, "WakerCommand::IsReady({id:?}))"),
            WakerCommand::Stop => write!(f, "WakerCommand::Stop()"),
        }
    }
}
