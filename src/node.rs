use crate::events::{self, EventSender};
use crate::network::{self, NetEvent, NetworkController, NetworkProcessor, PollWaker};
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

/// Event returned by [`NodeListener::for_each()`] and [`NodeListener::for_each_async()`]
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

/// Creates a node already working.
/// This function offers two instances: a [`NodeHandler`] to perform network and commands actions
/// and a [`NodeListener`] to receive the events the node receives.
///
/// Note that [`NodeListener`] is already listen for events from its creation.
/// In order to get the listened events you can call [`NodeListener::for_each()`]
/// Any event happened before `for_each()` call will be also dispatched.
///
/// # Examples
/// ```rust,no_run
/// use message_io::node::{self, NodeEvent};
///
/// let (handler, listener) = node::split();
///
/// listener.for_each(move |event| match event {
///     NodeEvent::Network(_) => { /* ... */ },
///     NodeEvent::Waker(command) => handler.stop(), //Received after 1 sec
/// });
/// ```
///
/// In case you don't use commands, specify the node type with an unit (`()`) type.
/// ```
/// use message_io::node::{self};
///
/// let (handler, listener) = node::split();
/// ```
pub fn split() -> (NodeHandler, NodeListener) {
    let (command_sender, command_receiver) = events::split();
    let (network_controller, network_processor) = network::split(command_sender, command_receiver);

    let running = AtomicBool::new(true);

    let handler = NodeHandler(Arc::new(NodeHandlerImpl { network: network_controller, running }));

    let listener = NodeListener::new(network_processor, handler.clone());

    (handler, listener)
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
    /// After this call, no more events will be processed by [`NodeListener::for_each()`].
    pub fn stop(&self) {
        self.0.running.store(false, Ordering::Relaxed);
    }

    /// Check if the node is running.
    /// Note that the node is running and listening events from its creation,
    /// not only once you call to [`NodeListener::for_each()`].
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

/// Listen events for network and command from user.
pub struct NodeListener {
    network_processor_opt: Option<NetworkProcessor>,
    handler: NodeHandler,
}

impl NodeListener {
    fn new(network_processor: NetworkProcessor, handler: NodeHandler) -> NodeListener {
        //
        NodeListener { network_processor_opt: Some(network_processor), handler }
    }

    /// Iterate indefinitely over all generated `NetEvent`.
    /// This function will work until [`NodeHandler::stop()`] is called.
    ///
    /// Note that any events generated before calling this function (e.g. some connection was done)
    /// will be stored and offered once you call `for_each()`.
    /// # Example
    /// ```rust,no_run
    /// use message_io::node::{self, NodeEvent};
    /// use message_io::network::Transport;
    ///
    /// let (handler, listener) = node::split();
    ///
    /// let (id, addr) = handler.network().listen(Transport::Tcp, "127.0.0.1:0").unwrap();
    ///
    /// listener.for_each(move |event| match event {
    ///     NodeEvent::Network(net_event) => { /* Your logic here */ },
    ///     NodeEvent::Waker(_) => handler.stop(),
    /// });
    /// // Blocked here until handler.stop() is called (1 sec).
    /// println!("Node is stopped");
    /// ```
    pub fn for_each(mut self, mut event_callback: impl FnMut(NodeEvent) + Send) {
        //
        crossbeam_utils::thread::scope(|_| {
            //
            let mut network_processor = self.network_processor_opt.take().unwrap();

            let cb = &mut |e| {
                if self.handler.is_running() {
                    event_callback(e);
                }
            };

            //
            while self.handler.is_running() {
                network_processor.process_poll_event(Some(*SAMPLING_TIMEOUT), cb);
            }
        })
        .unwrap();
    }

    /// Similar to [`NodeListener::for_each()`] but it returns the control to the user
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
    /// let (handler, listener) = node::split();
    ///
    /// let (id, addr) = handler.network().listen(Transport::Tcp, "127.0.0.1:0").unwrap();
    ///
    /// let task = listener.for_each_async(move |event| match event {
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
    pub fn for_each_async(
        mut self,
        mut event_callback: impl FnMut(NodeEvent) + Send + 'static,
    ) -> NodeTask {
        //
        let network_thread = {
            //
            let handler = self.handler.clone();

            //
            let mut network_processor = self.network_processor_opt.take().unwrap();

            //
            NamespacedThread::spawn("node-network-thread", move || {
                //
                let cb = &mut |e| {
                    if handler.is_running() {
                        event_callback(e);
                    }
                };
                //
                while handler.is_running() {
                    network_processor.process_poll_event(Some(*SAMPLING_TIMEOUT), cb);
                }
            })
        };

        NodeTask { network_thread }
    }
}

impl Drop for NodeListener {
    fn drop(&mut self) {
        //
    }
}

/// Entity used to ensure the lifetime of [`NodeListener::for_each_async()`] call.
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
    use super::*;
    use std::time::Duration;

    #[test]
    fn create_node_and_drop() {
        let (handler, _listener) = split();
        assert!(handler.is_running());
        // listener dropped here.
    }

    #[test]
    fn sync_node() {
        let (handler, listener) = split();
        assert!(handler.is_running());

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        listener.for_each(move |_| inner_handler.stop());

        assert!(!handler.is_running());
    }

    #[test]
    fn async_node() {
        let (handler, listener) = split();
        assert!(handler.is_running());

        let inner_handler = handler.clone();
        let _node_task = listener.for_each_async(move |event| match event.waker() {
            WakerCommand::Stop => inner_handler.stop(),
            _ => unreachable!(),
        });

        // Since here `NodeTask` is living, the node is considered running.
        assert!(handler.is_running());
        std::thread::sleep(Duration::from_millis(500));
        assert!(handler.is_running());
        handler.commands().post(handler.waker(), WakerCommand::Stop);
    }

    #[test]
    fn wait_task() {
        let (handler, listener) = split();

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        listener.for_each_async(move |_| inner_handler.stop()).wait();

        assert!(!handler.is_running());
    }

    #[test]
    fn wait_already_waited_task() {
        let (handler, listener) = split();

        handler.commands().post(handler.waker(), WakerCommand::Stop);

        let inner_handler = handler.clone();
        let mut task = listener.for_each_async(move |_| inner_handler.stop());
        assert!(handler.is_running());
        task.wait();
        assert!(!handler.is_running());
        task.wait();
        assert!(!handler.is_running());
    }
}
