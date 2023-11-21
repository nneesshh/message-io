use net_packet::NetPacketGuard;

use crate::network::Endpoint;

///
pub enum WakerCommand {
    Greet(String),
    Listen(Endpoint),
    Send(Endpoint, NetPacketGuard),
    SendTrunk, // for test only
    Stop,
}

impl std::fmt::Debug for WakerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WakerCommand::Greet(greet) => write!(f, "WakerCommand::Greet({greet:?})"),
            WakerCommand::Listen(endpoint) => write!(f, "WakerCommand::Listen({endpoint:?})"),
            WakerCommand::Send(endpoint, _) => write!(f, "WakerCommand::Send({endpoint:?})"),
            WakerCommand::SendTrunk => write!(f, "WakerCommand::SendTrunk()"),
            WakerCommand::Stop => write!(f, "WakerCommand::Stop()"),
        }
    }
}
