use std::sync::Arc;
use std::time::Duration;

use crate::network::{NetEvent, WakerCommand};
use crate::node::NodeHandler;

lazy_static::lazy_static! {
    static ref SAMPLING_TIMEOUT: Duration = Duration::from_millis(5);
}

/// Event returned by [`node_listener_for_each_async()`]
/// when some network event or signal is received.
pub enum NodeEvent {
    /// The `NetEvent` is an event that comes from the network.
    /// See [`NetEvent`] to know about the different network events.
    Network(NetEvent),

    /// The `WakerCommand` is a command from user and wake up the poll thread to recv.
    /// See [`WakerCommand`] to know about the waker commands.
    Waker(WakerCommand),
}

impl std::fmt::Debug for NodeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeEvent::Network(net_event) => write!(f, "NodeEvent::Network({net_event:?})"),
            NodeEvent::Waker(command) => write!(f, "NodeEvent::Waker({command:?})"),
        }
    }
}

impl NodeEvent {
    /// Assume the event is a [`NodeEvent::Network`], panics if not.
    pub fn network(self) -> NetEvent {
        match self {
            NodeEvent::Network(net_event) => net_event,
            NodeEvent::Waker(..) => panic!("NodeEvent must be a NetEvent"),
        }
    }
}

///
pub trait NodeEventEmitter {
    fn emit(&self, e: NodeEvent);
}

///
pub(crate) struct NodeEventEmitterImpl {
    handler: NodeHandler,
    callback: Arc<dyn Fn(&NodeHandler, NodeEvent) + Send + Sync>,
}

impl NodeEventEmitterImpl {
    ///
    pub fn new<F>(handler: &NodeHandler, callback: &Arc<F>) -> Self
    where
        F: Fn(&NodeHandler, NodeEvent) + Send + Sync + 'static,
    {
        Self {
            //
            handler: handler.clone(),
            callback: callback.clone(),
        }
    }
}

impl NodeEventEmitter for NodeEventEmitterImpl {
    #[inline(always)]
    fn emit(&self, e: NodeEvent) {
        (self.callback)(&self.handler, e);
    }
}
