use std::net::SocketAddr;
use std::io;

use net_packet::NetPacketGuard;

use crate::network::{Endpoint, ResourceId, TransportListen, RemoteAddr, TransportConnect};

///
pub enum WakerCommand {
    Greet(String),
    Connect(TransportConnect, RemoteAddr, Box<dyn FnOnce(io::Result<(Endpoint, SocketAddr)>) + Send>),
    Listen(TransportListen, SocketAddr, Box<dyn FnOnce(io::Result<(ResourceId, SocketAddr)>) + Send>),
    Send(Endpoint, NetPacketGuard),
    SendTrunk, // for test only
    Close(ResourceId),
    Stop,
}

impl std::fmt::Debug for WakerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WakerCommand::Greet(greet) => write!(f, "WakerCommand::Greet({greet:?})"),
            WakerCommand::Connect(_, addr, _) => write!(f, "WakerCommand::Connect({addr:?})"),
            WakerCommand::Listen(_, addr, _) => write!(f, "WakerCommand::Listen({addr:?})"),
            WakerCommand::Send(endpoint, _) => write!(f, "WakerCommand::Send({endpoint:?})"),
            WakerCommand::SendTrunk => write!(f, "WakerCommand::SendTrunk()"),
            WakerCommand::Close(id) => write!(f, "WakerCommand::Close({id:?}))"),
            WakerCommand::Stop => write!(f, "WakerCommand::Stop()"),
        }
    }
}
