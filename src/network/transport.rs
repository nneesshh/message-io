use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[cfg(feature = "tcp")]
use crate::adapters::tcp::tcp_driver::TcpDriver;
/*#[cfg(feature = "udp")]
use crate::adapters::udp::{self, UdpAdapter};

 */
#[cfg(feature = "ssl")]
use crate::adapters::ssl::ssl_driver::SslDriver;
/*#[cfg(feature = "websocket")]
use crate::adapters::ws::{self, WsAdapter};
 */

use crate::network::loader::BoxedEventProcessor;
use crate::node::NodeHandler;

use super::loader::EventProcessorList;
use super::resource_id::ResourceIdGenerator;
use super::MultiplexorWorkerParam;

/// Enum to identified the underlying transport used.
/// It can be passed to
/// [`NetworkController::connect()`](crate::network::NetworkController::connect()) and
/// [`NetworkController::listen()`](crate::network::NetworkController::connect()) methods
/// to specify the transport used.
#[derive(strum::EnumIter, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Transport {
    /// TCP protocol (available through the *tcp* feature).
    /// As stream protocol, receiving a message from TCP do not imply to read
    /// the entire message.
    /// If you want a packet based way to send over TCP, use `FramedTcp` instead.
    #[cfg(feature = "tcp")]
    Tcp,
    /*
    /// UDP protocol (available through the *udp* feature).
    /// Take into account that UDP is not connection oriented and a packet can be lost
    /// or received disordered.
    /// If it is specified in the listener and the address is a Ipv4 in the range of multicast ips
    /// (from `224.0.0.0` to `239.255.255.255`), the listener will be configured in multicast mode.
    #[cfg(feature = "udp")]
    Udp,

     */
    /// Tls protocol
    #[cfg(feature = "ssl")]
    Ssl,
    /*/// WebSocket protocol (available through the *websocket* feature).
    /// If you use a [`crate::network::RemoteAddr::Str`] in the `connect()` method,
    /// you can specify an URL with `wss` of `ws` schemas to connect with or without security.
    /// If you use a [`crate::network::RemoteAddr::Socket`] the socket will be a normal
    /// websocket with the following uri: `ws://{SocketAddr}/message-io-default`.
    #[cfg(feature = "websocket")]
    Ws,

     */
}

impl Transport {
    /// Associates an adapter.
    /// This method mounts the adapters to be used in the network instance.
    pub fn mount_adapter(
        self,
        param: &mut MultiplexorWorkerParam,
        node_handler: &NodeHandler,
        remote_id_generator: &Arc<ResourceIdGenerator>,
        local_id_generator: &Arc<ResourceIdGenerator>,
        processors: &mut EventProcessorList,
    ) {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp => {
                //
                let node_handler = node_handler.clone();
                let adapter_id = self.id();
                let poll = param.poll();
                let driver =
                    TcpDriver::new(remote_id_generator, local_id_generator, node_handler, poll);

                let index = adapter_id as usize;
                processors[index] = Box::new(driver) as BoxedEventProcessor;
            }
            /*#[cfg(feature = "udp")]
            Self::Udp => param.mount(self.id(), UdpAdapter, node_handler, remote_id_generator, local_id_generator, processors),

             */
            #[cfg(feature = "ssl")]
            Self::Ssl => {
                //
                let node_handler = node_handler.clone();
                let adapter_id = self.id();
                let poll = param.poll();

                let driver =
                    SslDriver::new(remote_id_generator, local_id_generator, node_handler, poll);

                let index = adapter_id as usize;
                processors[index] = Box::new(driver) as BoxedEventProcessor;
            } /*#[cfg(feature = "websocket")]
              Self::Ws => param.mount(self.id(), WsAdapter, node_handler, remote_id_generator, local_id_generator, processors),

               */
        };
    }

    /// Maximum theorical packet payload length available for each transport.
    ///
    /// The value returned by this function is the **theorical maximum** and could not be valid for
    /// all networks.
    /// You can ensure your message not exceeds `udp::MAX_INTERNET_PAYLOAD_LEN` in order to be
    /// more cross-platform compatible.
    pub const fn max_message_size(self) -> usize {
        match self {
            #[cfg(feature = "tcp")]
            Self::Tcp => usize::MAX,
            /*#[cfg(feature = "tcp")]
            Self::FramedTcp => usize::MAX,
            #[cfg(feature = "udp")]
            Self::Udp => udp::MAX_LOCAL_PAYLOAD_LEN,

             */
            #[cfg(feature = "ssl")]
            Self::Ssl => usize::MAX,
            /*#[cfg(feature = "websocket")]
            Self::Ws => ws::MAX_PAYLOAD_LEN,

             */
        }
    }

    /// Tell if the transport protocol is a connection oriented protocol.
    /// If it is, `Connection` and `Disconnection` events will be generated.
    pub const fn is_connection_oriented(self) -> bool {
        match self {
            #[cfg(feature = "tcp")]
            Transport::Tcp => true,
            /*#[cfg(feature = "tcp")]
            Transport::FramedTcp => true,
            #[cfg(feature = "udp")]
            Transport::Udp => false,

             */
            #[cfg(feature = "ssl")]
            Transport::Ssl => true,
            /*#[cfg(feature = "websocket")]
            Transport::Ws => true,

             */
        }
    }

    /// Tell if the transport protocol is a packet-based protocol.
    /// It implies that any send call corresponds to a data message event.
    /// It satisfies that the number of bytes sent are the same as received.
    /// The opossite of a packet-based is a stream-based transport (e.g Tcp).
    /// In this case, reading a data message event do not imply reading the entire message sent.
    /// It is in change of the user to determinate how to read the data.
    pub const fn is_packet_based(self) -> bool {
        match self {
            #[cfg(feature = "tcp")]
            Transport::Tcp => false,
            /*#[cfg(feature = "tcp")]
            Transport::FramedTcp => true,
            #[cfg(feature = "udp")]
            Transport::Udp => true,

             */
            #[cfg(feature = "ssl")]
            Transport::Ssl => true,
            /*#[cfg(feature = "websocket")]
            Transport::Ws => true,

             */
        }
    }

    /// Returns the adapter id used for this transport.
    /// It is equivalent to the position of the enum starting by 0
    pub const fn id(self) -> u8 {
        match self {
            #[cfg(feature = "tcp")]
            Transport::Tcp => 0,
            /*#[cfg(feature = "tcp")]
            Transport::FramedTcp => 1,
            #[cfg(feature = "udp")]
            Transport::Udp => 2,

             */
            #[cfg(feature = "ssl")]
            Transport::Ssl => 3,
            /*#[cfg(feature = "websocket")]
            Transport::Ws => 4,

             */
        }
    }
}

impl From<u8> for Transport {
    fn from(id: u8) -> Self {
        match id {
            #[cfg(feature = "tcp")]
            0 => Transport::Tcp,
            /*#[cfg(feature = "tcp")]
            1 => Transport::FramedTcp,
            #[cfg(feature = "udp")]
            2 => Transport::Udp,

             */
            #[cfg(feature = "ssl")]
            3 => Transport::Ssl,
            /*#[cfg(feature = "websocket")]
            4 => Transport::Ws,

             */
            _ => panic!("Not available transport"),
        }
    }
}

impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
