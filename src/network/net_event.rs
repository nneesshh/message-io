use net_packet::NetPacketGuard;

use super::endpoint::Endpoint;
use super::resource_id::ResourceId;

/// Enum used to describe a network event that an internal transport adapter has produced.
pub enum NetEvent {
    /// Connection result.
    /// This event is only generated after a [`crate::network::NetworkController::connect()`]
    /// call.
    /// The event contains the endpoint of the connection
    /// (same endpoint returned by the `connect()` method),
    /// and a boolean indicating the *result* of that connection.
    /// In *non connection-oriented transports* as *UDP* it simply means that the resource
    /// is ready to use, and the boolean will be always `true`.
    /// In connection-oriented transports it means that the handshake has been performed, and the
    /// connection is established and ready to use.
    /// Since this handshake could fail, the boolean could be `false`.
    Connected(Endpoint, bool),

    /// New endpoint has been accepted by a listener and considered ready to use.
    /// The event contains the resource id of the listener that accepted this connection.
    ///
    /// Note that this event will only be generated by connection-oriented transports as *TCP*.
    Accepted(Endpoint, ResourceId),

    /// Input message received by the network.
    /// In packet-based transports, the data of a message sent corresponds with the data of this
    /// event. This one-to-one relation is not conserved in stream-based transports as *TCP*.
    ///
    /// If you want a packet-based protocol over *TCP* use
    /// [`crate::network::Transport::FramedTcp`].
    Message(Endpoint, NetPacketGuard),

    /// This event is only dispatched when a connection is lost.
    /// Remove explicitely a resource will NOT generate the event.
    /// When this event is received, the resource is considered already removed,
    /// the user do not need to remove it after this event.
    /// A [`NetEvent::Message`] event will never be generated after this event from this endpoint.

    /// Note that this event will only be generated by connection-oriented transports as *TCP*.
    /// *UDP*, for example, is NOT connection-oriented, and the event can no be detected.
    Disconnected(Endpoint),
}

impl std::fmt::Debug for NetEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let string = match self {
            Self::Connected(endpoint, status) => format!("Connected({endpoint}, {status})"),
            Self::Accepted(endpoint, id) => format!("Accepted({endpoint}, {id})"),
            Self::Message(endpoint, data) => {
                format!("Message({}, {})", endpoint, data.buffer_raw_len())
            }
            Self::Disconnected(endpoint) => format!("Disconnected({endpoint})"),
        };
        write!(f, "NetEvent::{string}")
    }
}
