use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use net_packet::NetPacketGuard;
use pinky_swear::PinkySwear;

use crate::network::driver::{ConnectConfig, ListenConfig};
use crate::network::{
    self, Endpoint, Multiplexor, NetworkController, ResourceId, ResourceType, ToRemoteAddr,
    Transport, MULTIPLEXOR_THREAD_NUM,
};
use crate::network::{ResourceIdGenerator, WakerCommand};
use crate::node_event::{NodeEvent, NodeEventEmitterImpl};
use crate::util::thread::NamespacedThread;

lazy_static::lazy_static! {
    static ref SAMPLING_TIMEOUT: Duration = Duration::from_millis(5);
}

///
pub fn split() -> (Multiplexor, NodeHandler) {
    let mut mux = Multiplexor::default();
    let handler = create_node_handler(&mut mux);
    (mux, handler)
}

///
pub(crate) fn create_node_handler(mux: &mut Multiplexor) -> NodeHandler {
    //
    let running = AtomicBool::new(true);

    //
    NodeHandler(Arc::new(NodeHandlerImpl {
        remote_id_generator: mux.remote_id_generator.clone(),
        local_id_generator: mux.local_id_generator.clone(),

        controller_list: core::array::from_fn(|i| {
            let param = mux.worker_params[i].as_mut().unwrap();
            let command_sender = param.command_sender().clone();
            let waker = param.poll().create_waker();
            let controller = NetworkController::new(i + 70001, command_sender, waker);
            controller
        }),
        running,
    }))
}

#[inline(always)]
fn to_worker_index(resource_id: ResourceId) -> usize {
    1_usize + resource_id.raw() % MULTIPLEXOR_THREAD_NUM
}

struct NodeHandlerImpl {
    remote_id_generator: Arc<ResourceIdGenerator>,
    local_id_generator: Arc<ResourceIdGenerator>,

    controller_list: [NetworkController; MULTIPLEXOR_THREAD_NUM + 1],
    running: AtomicBool,
}

/// A shareable and clonable entity that allows to deal with
/// the network, send commands and stop the node.
pub struct NodeHandler(Arc<NodeHandlerImpl>);

impl NodeHandler {
    #[inline(always)]
    fn next_remote_id(&self, adapter_id: u8) -> ResourceId {
        self.0.remote_id_generator.generate(adapter_id)
    }

    #[inline(always)]
    fn next_local_id(&self, adapter_id: u8) -> ResourceId {
        self.0.local_id_generator.generate(adapter_id)
    }

    /// Creates a connection to the specified address.
    /// The endpoint, an identifier of the new connection, will be returned in the callback.
    /// This function will generate a [`NetEvent::Connected`] event with the result of the connection.
    /// This call will **NOT** block to perform the connection.
    ///
    /// Note that this function can return an error in the case the internal socket
    /// could not be binded or open in the OS, but never will return an error an regarding
    /// the connection itself.
    /// If you want to check if the connection has been established or not you have to read the
    /// boolean indicator in the [`NetEvent::Connected`] event.
    ///
    /// Example
    /// ```rust,no_run
    /// use net_packet::take_packet;
    ///
    /// use message_io::node_event::NodeEvent;
    /// use message_io::node;
    /// use message_io::network::{Transport, NetEvent};
    ///
    /// let (mut mux, handler) = node::split();
    ///
    /// handler.listen(Transport::Tcp, "127.0.0.1:0", move |h, ret| {
    ///     if let Ok((_, addr)) = ret {
    ///         h.connect(Transport::Tcp, addr, |_h, _| {});
    ///     }
    /// });
    ///
    /// let mut task = node::node_listener_for_each_async(mux, &handler, move |h, event| match event {
    ///     NodeEvent::Network(net_event) => match net_event {
    ///         NetEvent::Connected(endpoint, established) => {
    ///             if established {
    ///                 println!("Connected!");
    ///                 let buffer = take_packet(42);
    ///                 h.send(endpoint, buffer);
    ///             }
    ///             else {
    ///                 println!("Could not connect");
    ///             }
    ///         },
    ///         NetEvent::Accepted(endpoint, listening_id) => {
    ///             println!("New connected endpoint: {}", endpoint.addr());
    ///         },
    ///         _ => (),
    ///     }
    ///     NodeEvent::Waker(_) => h.stop(),
    /// });
    /// ```
    pub fn connect<F>(&self, transport: Transport, addr: impl ToRemoteAddr, cb: F)
    where
        F: FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send + 'static,
    {
        let config = ConnectConfig::default();
        self.connect_with(config, transport, addr, cb)
    }

    /// Creates a connection to the specified address with custom transport options for transports
    /// that support it.
    /// The endpoint, an identifier of the new connection, will be returned in the callback.
    /// This function will generate a [`NetEvent::Connected`] event with the result of the
    /// connection.  This call will **NOT** block to perform the connection.
    ///
    /// Note that this function can return an error in the case the internal socket
    /// could not be binded or open in the OS, but never will return an error regarding
    /// the connection itself.
    /// If you want to check if the connection has been established or not you have to read the
    /// boolean indicator in the [`NetEvent::Connected`] event.
    ///
    /// Example
    /// ```rust,no_run
    /// use net_packet::take_packet;
    ///
    /// use message_io::node_event::NodeEvent;
    /// use message_io::node;
    /// use message_io::network::{NetEvent, Transport};
    /// use message_io::network::driver::{ConnectConfig};
    ///
    /// let (mut mux, handler) = node::split();
    ///
    /// let config = ConnectConfig::default();
    /// let addr = "127.0.0.1:7878";
    /// handler.connect_with(config, Transport::Tcp, addr, |_h, _| {});
    ///
    /// let mut task = node::node_listener_for_each_async(mux, &handler, move |h, event| match event {
    ///     NodeEvent::Network(net_event) => match net_event {
    ///         NetEvent::Connected(endpoint, established) => {
    ///             if established {
    ///                 println!("Connected!");
    ///                 let buffer = take_packet(42);
    ///                 h.send(endpoint, buffer);
    ///             }
    ///             else {
    ///                 println!("Could not connect");
    ///             }
    ///         },
    ///         _ => (),
    ///     }
    ///     NodeEvent::Waker(_) => h.stop(),
    /// });
    /// ```
    pub fn connect_with<F>(
        &self,
        config: ConnectConfig,
        transport: Transport,
        addr: impl ToRemoteAddr,
        cb: F,
    ) where
        F: FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send + 'static,
    {
        let adapter_id = transport.id();
        let remote_id = self.next_remote_id(adapter_id);
        let addr = addr.to_remote_addr().unwrap();

        // post to first thread
        self.post0(WakerCommand::Connect(config, remote_id, addr, Box::new(cb)));
    }

    /// Note that the `Connect` event will be also generated.
    ///
    /// Since this function blocks the current thread, it must NOT be used inside
    /// the network callback because the internal event could not be processed.
    ///
    /// In order to get the best scalability and performance, use the non-blocking
    /// [`NetworkController::connect()`] version.
    ///
    /// Example
    /// ```rust,no_run
    /// use net_packet::take_packet;
    ///
    /// use message_io::node;
    /// use message_io::network::{Transport, NetEvent};
    ///
    /// let (mut mux, handler) = node::split();
    ///
    /// // poll network event
    /// let mut task = node::node_listener_for_each_async(mux, &handler, move |_h, _| {});
    ///
    /// let (_, addr) = handler.listen_sync(Transport::Tcp, "127.0.0.1:0").unwrap();
    /// match handler.connect_sync(Transport::Tcp, addr) {
    ///     Ok((endpoint, _)) => {
    ///         println!("Connected!");
    ///         let buffer = take_packet(42);
    ///         handler.send(endpoint, buffer);
    ///     }
    ///     Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
    ///         println!("Could not connect");
    ///     }
    ///     Err(err) => println!("An OS error creating the socket"),
    /// }
    /// task.wait();
    /// ```
    pub fn connect_sync(
        &self,
        transport: Transport,
        addr: impl ToRemoteAddr,
    ) -> io::Result<(Endpoint, SocketAddr)> {
        let config = ConnectConfig::default();
        self.connect_sync_with(config, transport, addr)
    }

    /// Creates a connection to the specified address with custom transport options for transports
    /// that support it.
    /// This function is similar to [`NetworkController::connect_with()`] but will block
    /// until for the connection is ready.
    /// If the connection can not be established, a `ConnectionRefused` error will be returned.
    ///
    /// Note that the `Connect` event will be also generated.
    ///
    /// Since this function blocks the current thread, it must NOT be used inside
    /// the network callback because the internal event could not be processed.
    ///
    /// In order to get the best scalability and performance, use the non-blocking
    /// [`NetworkController::connect_with()`] version.
    ///
    /// Example
    /// ```rust,no_run
    /// use net_packet::take_packet;
    ///
    /// use message_io::node;
    /// use message_io::network::{NetEvent, Transport};
    /// use message_io::network::driver::{ConnectConfig};
    ///
    /// let (mut mux, handler) = node::split();
    ///
    /// // poll network event
    /// let mut task = node::node_listener_for_each_async(mux, &handler, move |_h, _| {});
    ///
    /// let config = ConnectConfig::default();
    /// let addr = "127.0.0.1:7878";
    /// match handler.connect_sync_with(config, Transport::Tcp, addr) {
    ///     Ok((endpoint, _)) => {
    ///         println!("Connected!");
    ///         let buffer = take_packet(42);
    ///         handler.send(endpoint, buffer);
    ///     }
    ///     Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
    ///         println!("Could not connect");
    ///     }
    ///     Err(err) => println!("An OS error creating the socket"),
    /// }
    /// task.wait();
    /// ```
    pub fn connect_sync_with(
        &self,
        config: ConnectConfig,
        transport: Transport,
        addr: impl ToRemoteAddr,
    ) -> io::Result<(Endpoint, SocketAddr)> {
        let (promise, pinky) = PinkySwear::<io::Result<(Endpoint, SocketAddr)>>::new();
        let cb = move |_h: &NodeHandler, ret| {
            //
            pinky.swear(ret);
        };

        let addr = addr.to_remote_addr().unwrap();
        self.connect_with(config, transport, addr, cb);
        let ret = promise.wait();
        match ret {
            Ok((endpoint, addr)) => {
                // about sleep 1 second
                for _ in 0..1000 {
                    std::thread::sleep(Duration::from_millis(1));
                    let id = endpoint.resource_id();
                    match self.is_ready_sync(id) {
                        Some(true) => {
                            //
                            return Ok((endpoint, addr));
                        }
                        Some(false) => {
                            //
                            continue;
                        }
                        None => {
                            //
                            return Err(io::Error::new(
                                io::ErrorKind::ConnectionRefused,
                                "Connection refused",
                            ));
                        }
                    }
                }

                // abort after retry 1000 times
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Connection aborted"));
            }
            Err(err) => {
                //
                return Err(err);
            }
        };
    }

    /// Listen messages from specified transport.
    /// The given address will be used as interface and listening port.
    /// If the port can be opened, a [ResourceId] identifying the listener is returned
    /// along with the local address, or an error if not.
    /// The address is returned despite you passed as parameter because
    /// when a `0` port is specified, the OS will give choose the value.
    pub fn listen<F>(&self, transport: Transport, addr: impl ToSocketAddrs, cb: F)
    where
        F: FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send + 'static,
    {
        let config = ListenConfig::default();
        self.listen_with(config, transport, addr, cb);
    }

    /// Listen messages from specified transport with custom transport options for transports that
    /// support it.
    /// The given address will be used as interface and listening port.
    /// If the port can be opened, a [ResourceId] identifying the listener is returned
    /// along with the local address, or an error if not.
    /// The address is returned despite you passed as parameter because
    /// when a `0` port is specified, the OS will give choose the value.
    pub fn listen_with<F>(
        &self,
        config: ListenConfig,
        transport: Transport,
        addr: impl ToSocketAddrs,
        cb: F,
    ) where
        F: FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send + 'static,
    {
        let adapter_id = transport.id();
        let local_id = self.next_local_id(adapter_id);
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();

        // post to the first thread
        self.post0(WakerCommand::Listen(config, local_id, addr, Box::new(cb)));
    }

    ///
    pub fn listen_sync(
        &self,
        transport: Transport,
        addr: impl ToSocketAddrs,
    ) -> io::Result<(ResourceId, SocketAddr)> {
        let config = ListenConfig::default();
        self.listen_sync_with(config, transport, addr)
    }

    ///
    pub fn listen_sync_with(
        &self,
        config: ListenConfig,
        transport: Transport,
        addr: impl ToSocketAddrs,
    ) -> io::Result<(ResourceId, SocketAddr)> {
        let (promise, pinky) = PinkySwear::<io::Result<(ResourceId, SocketAddr)>>::new();
        let cb = move |_h: &NodeHandler, ret| {
            //
            pinky.swear(ret);
        };
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        self.listen_with(config, transport, addr, cb);
        promise.wait()
    }

    /// Send the data message thought the connection represented by the given endpoint.
    /// This function returns a [`SendStatus`] indicating the status of this send.
    /// There is no guarantee that send over a correct connection generates a [`SendStatus::Sent`]
    /// because any time a connection can be disconnected (even while you are sending).
    /// Except cases where you need to be sure that the message has been sent,
    /// you will want to process a [`NetEvent::Disconnected`] to determine if the connection +
    /// is *alive* instead of check if `send()` returned [`SendStatus::ResourceNotFound`].
    #[inline(always)]
    pub fn send(&self, endpoint: Endpoint, buffer: NetPacketGuard) {
        let id = endpoint.resource_id();
        self.post(id, WakerCommand::Send(endpoint, buffer));
    }

    ///
    #[inline(always)]
    pub fn send_sync(&self, endpoint: Endpoint, buffer: NetPacketGuard) {
        let (promise, pinky) = PinkySwear::<()>::new();
        let cb = move || {
            //
            pinky.swear(());
        };
        let id = endpoint.resource_id();
        self.post(id, WakerCommand::SendSync(endpoint, buffer, Box::new(cb)));
        promise.wait();
    }

    /// Check a resource specified by `resource_id` is ready.
    /// If the status is `true` means that the resource is ready to use.
    /// In connection oriented transports, it implies the resource is connected.
    /// If the status is `false` it means that the resource is not yet ready to use.
    /// If the resource has been removed, disconnected, or does not exists in the network,
    /// a `None` is returned.
    pub fn is_ready_sync(&self, id: ResourceId) -> Option<bool> {
        let (promise, pinky) = PinkySwear::<Option<bool>>::new();
        let cb = move |_h: &NodeHandler, opt| {
            //
            pinky.swear(opt);
        };
        self.post(id, WakerCommand::IsReady(id, Box::new(cb)));
        promise.wait()
    }

    /// Remove a network resource.
    /// Returns `false` if the resource id doesn't exists.
    /// This is used to remove resources as connection or listeners.
    /// Resources of endpoints generated by listening in connection oriented transports
    /// can also be removed to close the connection.
    /// Removing an already connected connection implies a disconnection.
    /// Note that non-oriented connections as UDP use its listener resource to manage all
    /// remote endpoints internally, the remotes have not resource for themselfs.
    /// It means that all generated `Endpoint`s share the `ResourceId` of the listener and
    /// if you remove this resource you are removing the listener of all of them.
    /// For that cases there is no need to remove the resource because non-oriented connections
    /// have not connection itself to close, 'there is no spoon'.
    #[inline(always)]
    pub fn close(&self, id: ResourceId) {
        match id.resource_type() {
            ResourceType::Local => {
                // controller of listener is at the first thread
                self.controller0().do_post(WakerCommand::Close(id));
            }
            ResourceType::Remote => {
                // thread by id
                self.controller(id).do_post(WakerCommand::Close(id));
            }
        }
    }

    /// Finalizes the [`NodeTask`].
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

    ///
    #[inline(always)]
    pub fn post(&self, id: ResourceId, command: WakerCommand) {
        self.controller(id).do_post(command);
    }

    ///
    #[inline(always)]
    pub fn post0(&self, command: WakerCommand) {
        self.controller0().do_post(command);
    }

    #[inline(always)]
    fn controller(&self, resource_id: ResourceId) -> &NetworkController {
        let idx = to_worker_index(resource_id);
        &self.0.controller_list[idx]
    }

    #[inline(always)]
    fn controller0(&self) -> &NetworkController {
        &self.0.controller_list[0]
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
/// use message_io::node_event::NodeEvent;
/// use message_io::node;
/// use message_io::network::Transport;
///
/// let (mut mux, handler) = node::split();
///
/// let handler2 = handler.clone();
/// let _ = std::thread::spawn(move || {
///     let (id, addr) = handler2.listen_sync(Transport::Tcp, "127.0.0.1:0").unwrap();
/// });
///
/// let task = node::node_listener_for_each_async(mux, &handler, move |h, event| match event {
///      NodeEvent::Network(net_event) => { /* Your logic here */ },
///      NodeEvent::Waker(_) => h.stop(),
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
    mut mux: Multiplexor,
    handler: &NodeHandler,
    event_callback: F,
) -> NodeTask
where
    F: Fn(&NodeHandler, NodeEvent) + Send + Sync + 'static,
{
    let mut task = NodeTask { network_thread_vec: Vec::new() };
    let event_callback = Arc::new(event_callback);

    for i in 0..mux.worker_params.len() {
        //
        let emitter = NodeEventEmitterImpl::new(handler, &event_callback);
        let thr = launch_worker_thread(&mut mux, i, handler, emitter);
        task.network_thread_vec.push(thr);
    }

    task
}

fn launch_worker_thread(
    mux: &mut Multiplexor,
    index: usize,
    handler: &NodeHandler,
    mut emitter: NodeEventEmitterImpl,
) -> NamespacedThread<()> {
    //
    let network_thread = {
        //
        assert!(index < mux.worker_params.len());
        let param = mux.worker_params[index].take().unwrap();

        //
        let remote_id_generator = mux.remote_id_generator.clone();
        let local_id_generator = mux.local_id_generator.clone();

        //
        let handler = handler.clone();
        NamespacedThread::spawn("multiplexor-worker-thread", move || {
            log::info!("launch multiplexor-worker-thread: {index}");

            //
            let rgen = remote_id_generator.clone();
            let lgen = local_id_generator.clone();
            let mut worker = network::create_multiplexor_worker(param, &handler, &rgen, &lgen);

            while handler.is_running() {
                worker.process_poll_event(Some(*SAMPLING_TIMEOUT), &mut emitter);
            }
        })
    };

    network_thread
}

/// Entity used to ensure the lifetime of [`node_listener_for_each_async()`] call.
/// The node will process events asynchronously while this entity lives.
/// The destruction of this entity will block until the task is finished.
/// If you want to "unblock" the thread that drops this entity call to
/// [`NodeHandler::stop()`] before or from another thread.
pub struct NodeTask {
    network_thread_vec: Vec<NamespacedThread<()>>,
}

impl NodeTask {
    /// Block the current thread until the task finalizes.
    /// To finalize the task call [`NodeHandler::stop()`].
    /// Calling `wait()` over an already finished task do not block.
    pub fn wait(&mut self) {
        for thr in &mut self.network_thread_vec {
            thr.try_join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::node;

    use super::*;

    #[test]
    fn sync_node() {
        let (mux, handler) = split();
        assert!(handler.is_running());

        handler.post0(WakerCommand::Stop);

        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, _| h.stop());

        task.wait();
        assert!(!handler.is_running());
    }

    #[test]
    fn async_node() {
        let (mux, handler) = split();
        assert!(handler.is_running());

        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, e| {
            match e {
                NodeEvent::Network(_) => {}
                NodeEvent::Waker(command) => {
                    //
                    match command {
                        WakerCommand::Stop => h.stop(),
                        _ => {}
                    }
                }
            }
        });

        // Since here `NodeTask` is living, the node is considered running.
        assert!(handler.is_running());
        std::thread::sleep(Duration::from_millis(500));
        assert!(handler.is_running());
        handler.post0(WakerCommand::Stop);
        task.wait();
    }

    #[test]
    fn wait_task() {
        let (mux, handler) = split();

        handler.post0(WakerCommand::Stop);

        node::node_listener_for_each_async(mux, &handler, move |h, _| h.stop()).wait();

        assert!(!handler.is_running());
    }

    #[test]
    fn wait_already_waited_task() {
        let (mux, handler) = split();

        handler.post0(WakerCommand::Stop);

        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, _| h.stop());
        task.wait();
        assert!(!handler.is_running());
        task.wait();
        assert!(!handler.is_running());
    }
}
