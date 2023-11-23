use crate::events::EventSender;
use crate::network::{self, NetEvent, NetworkController, PollEngine, PollWaker};
use crate::util::thread::NamespacedThread;
use crate::WakerCommand;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

lazy_static::lazy_static! {
    static ref SAMPLING_TIMEOUT: Duration = Duration::from_millis(5);
}

/// Event returned by [`node_listener_for_each_async()`]
/// when some network event or signal is received.
pub enum NodeEvent {
    /// The `NodeEvent` is an event that comes from the network.
    /// See [`NetEvent`] to know about the different network events.
    Network(NetEvent),

    /// The `NodeEvent` is a command from user and wake up the poll thread to recv.
    /// See [`EventSender`] to know about how to send commands.
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

    /// Assume the event is a [`NodeEvent::Waker`], panics if not.
    pub fn waker(self) -> WakerCommand {
        match self {
            NodeEvent::Network(..) => panic!("NodeEvent must be a WakerCommand"),
            NodeEvent::Waker(command) => command,
        }
    }
}

///
pub fn split() -> (PollEngine, NodeHandler) {
    let engine = PollEngine::default();
    let handler = create_node_handler(&engine);
    (engine, handler)
}

///
pub fn create_node_handler(engine: &PollEngine) -> NodeHandler {
    let command_sender = engine.command_sender().clone();
    let waker = engine.poll().create_waker();
    let controller = NetworkController::new(command_sender, waker);
    let running = AtomicBool::new(true);
    NodeHandler(Arc::new(NodeHandlerImpl { network: controller, running }))
}

struct NodeHandlerImpl {
    network: NetworkController,
    running: AtomicBool,
}

/// A shareable and clonable entity that allows to deal with
/// the network, send commands and stop the node.
pub struct NodeHandler(Arc<NodeHandlerImpl>);

impl NodeHandler {
    /// Returns a reference to the NetworkController to deal with the network.
    /// See [`NetworkController`]
    #[inline(always)]
    pub fn network(&self) -> &NetworkController {
        &self.0.network
    }

    /// Returns a reference to the EventSender to send commands to the node.
    /// Signals are events that the node send to itself useful in situation where you need
    /// to "wake up" the [`NodeListener`] to perform some action.
    /// See [`EventSender`].
    #[inline(always)]
    pub fn commands(&self) -> &EventSender<WakerCommand> {
        self.network().commands()
    }

    ///
    #[inline(always)]
    pub fn waker(&self) -> &PollWaker {
        &self.network().waker()
    }

    /// Finalizes the [`NodeListener`].
    /// After this call, no more events will be processed by [`node_listener_for_each_async()`].
    pub fn stop(&self) {
        self.0.running.store(false, Ordering::Relaxed);
    }

    /// Check if the node is running.
    /// Note that the node is running and listening events from its creation,
    /// not only once you call to [`node_listener_for_each_async()`].
    #[inline(always)]
    pub fn is_running(&self) -> bool {
        self.0.running.load(Ordering::Relaxed)
    }
}

impl Clone for NodeHandler {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// NodeListener: Listen events for network and command from user.
/// It iterates indefinitely over all generated `NetEvent`, and returns the control to the user
/// after calling it. The events will be processed asynchronously.
/// A `NodeTask` representing this asynchronous job is returned.
/// Destroying this object will result in blocking the current thread until
/// [`NodeHandler::stop()`] is called.
///
/// In order to allow the node working asynchronously, you can move the `NodeTask` to a
/// an object with a longer lifetime.
///
/// # Example
/// ```rust,no_run
/// use message_io::node::{self, NodeEvent};
/// use message_io::network::Transport;
///
/// let (engine, handler) = node::split();
///
/// let (id, addr) = handler.network().listen(Transport::Tcp, "127.0.0.1:0").unwrap();
///
/// let task = node::node_listener_for_each_async(engine, handler, move |event| match event {
///      NodeEvent::Network(net_event) => { /* Your logic here */ },
///      NodeEvent::Waker(_) => handler.stop(),
/// });
/// // for_each_async() will act asynchronous during 'task' lifetime.
///
/// // ...
/// println!("Node is running");
/// // ...
///
/// drop(task); // Blocked here until handler.stop() is called (1 sec).
/// // Also task.wait(); can be called doing the same (but taking a mutable reference).
///
/// println!("Node is stopped");
/// ```
pub fn node_listener_for_each_async<F>(
    engine: PollEngine,
    handler: &NodeHandler,
    mut event_callback: F,
) -> NodeTask
where
    F: FnMut(NodeEvent) + Send + 'static,
{
    //
    let handler = handler.clone();
    let network_thread = {
        //
        NamespacedThread::spawn("node-network-thread", move || {
            //
            let mut network_processor = network::create_processor(engine);

            //
            let handler2 = handler.clone();
            let cb = &mut |e| {
                if handler2.is_running() {
                    event_callback(e);
                }
            };
            while handler.is_running() {
                network_processor.process_poll_event(Some(*SAMPLING_TIMEOUT), cb);
            }
        })
    };

    NodeTask { network_thread }
}

/// Entity used to ensure the lifetime of [`node_listener_for_each_async()`] call.
/// The node will process events asynchronously while this entity lives.
/// The destruction of this entity will block until the task is finished.
/// If you want to "unblock" the thread that drops this entity call to
/// [`NodeHandler::stop()`] before or from another thread.
#[must_use = "The NodeTask must be used or the asynchronous task will be dropped in return"]
pub struct NodeTask {
    network_thread: NamespacedThread<()>,
}

impl NodeTask {
    /// Block the current thread until the task finalizes.
    /// Similar to call `drop(node_task)` but more verbose and without take the ownership.
    /// To finalize the task call [`NodeHandler::stop()`].
    /// Calling `wait()` over an already finished task do not block.
    pub fn wait(&mut self) {
        self.network_thread.try_join();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::node;

    use super::*;

    #[test]
    fn sync_node() {
        let (engine, handler) = split();
        assert!(handler.is_running());

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        let mut task =
            node::node_listener_for_each_async(engine, &handler, move |_| inner_handler.stop());

        task.wait();
        assert!(!handler.is_running());
    }

    #[test]
    fn async_node() {
        let (engine, handler) = split();
        assert!(handler.is_running());

        let inner_handler = handler.clone();
        let mut task =
            node::node_listener_for_each_async(engine, &handler, move |event| {
                match event.waker() {
                    WakerCommand::Stop => inner_handler.stop(),
                    _ => unreachable!(),
                }
            });

        // Since here `NodeTask` is living, the node is considered running.
        assert!(handler.is_running());
        std::thread::sleep(Duration::from_millis(500));
        assert!(handler.is_running());
        handler.commands().post(handler.waker(), WakerCommand::Stop);
        task.wait();
    }

    #[test]
    fn wait_task() {
        let (engine, handler) = split();

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        node::node_listener_for_each_async(engine, &handler, move |_| inner_handler.stop()).wait();

        assert!(!handler.is_running());
    }

    #[test]
    fn wait_already_waited_task() {
        let (engine, handler) = split();

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        let mut task =
            node::node_listener_for_each_async(engine, &handler, move |_| inner_handler.stop());
        assert!(handler.is_running());
        task.wait();
        assert!(!handler.is_running());
        task.wait();
        assert!(!handler.is_running());
    }
}
