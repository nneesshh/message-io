mod driver;
mod endpoint;
mod loader;
mod poll;
mod registry;
mod remote_addr;
mod resource_id;
mod transport;

/// Module that specify the pattern to follow to create adapters.
/// This module is not part of the public API itself,
/// it must be used from the internals to build new adapters.
pub mod adapter;

// Reexports
pub use adapter::SendStatus;
pub use driver::NetEvent;
pub use endpoint::Endpoint;
pub use loader::PollEngine;
pub use poll::PollWaker;
pub use remote_addr::{RemoteAddr, ToRemoteAddr};
pub use resource_id::{ResourceId, ResourceType};
pub use transport::{Transport, TransportConnect, TransportListen};

use loader::{EventProcessorList, Processor, UnimplementedDriver};
use poll::{Poll, PollEvent};

use std::io::{self};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use net_packet::NetPacketGuard;
use pinky_swear::PinkySwear;
use strum::IntoEnumIterator;

use crate::WakerCommand;

use super::events::{EventReceiver, EventSender};
use super::node::NodeEvent;

/// Create a network processor instance.
pub fn create_processor(mut engine: PollEngine) -> NetworkProcessor {
    let command_receiver = engine.command_receiver().clone();
    let mut processors: EventProcessorList =
        (0..ResourceId::MAX_ADAPTERS).map(|_| Box::new(UnimplementedDriver) as Processor).collect();
    Transport::iter().for_each(|transport| transport.mount_adapter(&mut engine, &mut processors));
    NetworkProcessor::new(engine.take(), command_receiver, processors)
}

/// Shareable instance in charge of control all the connections.
pub struct NetworkController {
    commands: EventSender<WakerCommand>,
    waker: PollWaker,
}

impl NetworkController {
    ///
    pub fn new(commands: EventSender<WakerCommand>, waker: PollWaker) -> NetworkController {
        Self { commands, waker }
    }

    /// Command queue
    #[inline(always)]
    pub fn commands(&self) -> &EventSender<WakerCommand> {
        &self.commands
    }

    /// Poll waker
    #[inline(always)]
    pub fn waker(&self) -> &PollWaker {
        &self.waker
    }

    /// Creates a connection to the specified address.
    /// The endpoint, an identifier of the new connection, will be returned.
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
    /// use message_io::node::{self, NodeEvent};
    /// use message_io::network::{Transport, NetEvent};
    ///
    /// let (engine, handler) = node::split();
    ///
    /// let (id, addr) = handler.network().listen(Transport::Tcp, "127.0.0.1:0").unwrap();
    /// let (conn_endpoint, _) = handler.network().connect(Transport::Tcp, addr).unwrap();
    /// // The socket could not be able to send yet.
    ///
    /// listener.for_each(move |event| match event {
    ///     NodeEvent::Network(net_event) => match net_event {
    ///         NetEvent::Connected(endpoint, established) => {
    ///             assert_eq!(conn_endpoint, endpoint);
    ///             if established {
    ///                 println!("Connected!");
    ///                 let buffer = take_packet(42);
    ///                 handler.network().send(endpoint, buffer);
    ///             }
    ///             else {
    ///                 println!("Could not connect");
    ///             }
    ///         },
    ///         NetEvent::Accepted(endpoint, listening_id) => {
    ///             assert_eq!(id, listening_id);
    ///             println!("New connected endpoint: {}", endpoint.addr());
    ///         },
    ///         _ => (),
    ///     }
    ///     NodeEvent::Waker(_) => handler.stop(),
    /// });
    /// ```
    pub fn connect<F>(&self, transport: Transport, addr: impl ToRemoteAddr, cb: F)
    where
        F: FnOnce(io::Result<(Endpoint, SocketAddr)>) + Send + 'static,
    {
        self.connect_with(transport.into(), addr, cb);
    }

    /// Creates a connection to the specified address with custom transport options for transports
    /// that support it.
    /// The endpoint, an identifier of the new connection, will be returned.
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
    /// use message_io::node::{self, NodeEvent};
    /// use message_io::network::{TransportConnect, NetEvent};
    /// use message_io::adapters::tcp::{TcpConnectConfig};
    ///
    /// let (engine, handler) = node::split();
    ///
    /// let config = TcpConnectConfig::default();
    /// let addr = "127.0.0.1:7878";
    /// let (conn_endpoint, _) = handler.network().connect_with(TransportConnect::Tcp(config), addr).unwrap();
    /// // The socket could not be able to send yet.
    ///
    /// listener.for_each(move |event| match event {
    ///     NodeEvent::Network(net_event) => match net_event {
    ///         NetEvent::Connected(endpoint, established) => {
    ///             assert_eq!(conn_endpoint, endpoint);
    ///             if established {
    ///                 println!("Connected!");
    ///                 let buffer = take_packet(42);
    ///                 handler.network().send(endpoint, buffer);
    ///             }
    ///             else {
    ///                 println!("Could not connect");
    ///             }
    ///         },
    ///         _ => (),
    ///     }
    ///     NodeEvent::Waker(_) => handler.stop(),
    /// });
    /// ```
    pub fn connect_with<F>(
        &self,
        transport_connect: TransportConnect,
        addr: impl ToRemoteAddr,
        cb: F,
    ) where
        F: FnOnce(io::Result<(Endpoint, SocketAddr)>) + Send + 'static,
    {
        let addr = addr.to_remote_addr().unwrap();
        self.commands
            .post(self.waker(), WakerCommand::Connect(transport_connect, addr, Box::new(cb)));
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
    /// use message_io::node::{self, NodeEvent};
    /// use message_io::network::{Transport, NetEvent};
    ///
    /// let (handler, listener) = node::split();
    ///
    /// // poll network event
    /// let _node_task = listener.for_each_async(move |_event| {});
    ///
    /// let (_, addr) = handler.network().listen(Transport::Tcp, "127.0.0.1:0").unwrap();
    /// match handler.network().connect_sync(Transport::Tcp, addr) {
    ///     Ok((endpoint, _)) => {
    ///         println!("Connected!");
    ///         let buffer = take_packet(42);
    ///         handler.network().send(endpoint, buffer);
    ///     }
    ///     Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
    ///         println!("Could not connect");
    ///     }
    ///     Err(err) => println!("An OS error creating the socket"),
    /// }
    /// ```
    pub fn connect_sync(
        &self,
        transport: Transport,
        addr: impl ToRemoteAddr,
    ) -> io::Result<(Endpoint, SocketAddr)> {
        self.connect_sync_with(transport.into(), addr)
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
    /// use message_io::node::{self, NodeEvent};
    /// use message_io::network::{TransportConnect, NetEvent};
    /// use message_io::adapters::tcp::{TcpConnectConfig};
    ///
    /// let (handler, listener) = node::split();
    ///
    /// // poll network event
    /// let _node_task = listener.for_each_async(move |_event| {});
    ///
    /// let config = TcpConnectConfig::default();
    /// let addr = "127.0.0.1:7878";
    /// match handler.network().connect_sync_with(TransportConnect::Tcp(config), addr) {
    ///     Ok((endpoint, _)) => {
    ///         println!("Connected!");
    ///         let buffer = take_packet(42);
    ///         handler.network().send(endpoint, buffer);
    ///     }
    ///     Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
    ///         println!("Could not connect");
    ///     }
    ///     Err(err) => println!("An OS error creating the socket"),
    /// }
    /// ```
    pub fn connect_sync_with(
        &self,
        transport_connect: TransportConnect,
        addr: impl ToRemoteAddr,
    ) -> io::Result<(Endpoint, SocketAddr)> {
        let (promise, pinky) = PinkySwear::<io::Result<(Endpoint, SocketAddr)>>::new();
        let cb = move |ret| {
            //
            pinky.swear(ret);
        };
        let addr = addr.to_remote_addr().unwrap();
        self.commands
            .post(self.waker(), WakerCommand::Connect(transport_connect, addr, Box::new(cb)));
        promise.wait()
    }

    /// Listen messages from specified transport.
    /// The given address will be used as interface and listening port.
    /// If the port can be opened, a [ResourceId] identifying the listener is returned
    /// along with the local address, or an error if not.
    /// The address is returned despite you passed as parameter because
    /// when a `0` port is specified, the OS will give choose the value.
    pub fn listen<F>(&self, transport: Transport, addr: impl ToSocketAddrs, cb: F)
    where
        F: FnOnce(io::Result<(ResourceId, SocketAddr)>) + Send + 'static,
    {
        self.listen_with(transport.into(), addr, cb);
    }

    /// Listen messages from specified transport with custom transport options for transports that
    /// support it.
    /// The given address will be used as interface and listening port.
    /// If the port can be opened, a [ResourceId] identifying the listener is returned
    /// along with the local address, or an error if not.
    /// The address is returned despite you passed as parameter because
    /// when a `0` port is specified, the OS will give choose the value.

    pub fn listen_with<F>(&self, transport_listen: TransportListen, addr: impl ToSocketAddrs, cb: F)
    where
        F: FnOnce(io::Result<(ResourceId, SocketAddr)>) + Send + 'static,
    {
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        self.commands
            .post(self.waker(), WakerCommand::Listen(transport_listen, addr, Box::new(cb)));
    }

    ///
    pub fn listen_sync(
        &self,
        transport: Transport,
        addr: impl ToSocketAddrs,
    ) -> io::Result<(ResourceId, SocketAddr)> {
        self.listen_sync_with(transport.into(), addr)
    }

    ///
    pub fn listen_sync_with(
        &self,
        transport_listen: TransportListen,
        addr: impl ToSocketAddrs,
    ) -> io::Result<(ResourceId, SocketAddr)> {
        let (promise, pinky) = PinkySwear::<io::Result<(ResourceId, SocketAddr)>>::new();
        let cb = move |ret| {
            //
            pinky.swear(ret);
        };
        let addr = addr.to_socket_addrs().unwrap().next().unwrap();
        self.commands
            .post(self.waker(), WakerCommand::Listen(transport_listen, addr, Box::new(cb)));
        promise.wait()
    }

    /// Send the data in buffer to the poll thread
    /// This function returns nothing.
    #[inline(always)]
    pub fn send(&self, endpoint: Endpoint, buffer: NetPacketGuard) {
        self.commands.post(self.waker(), WakerCommand::Send(endpoint, buffer));
    }

    /// Remove a network resource.
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
    //#[inline(always)]
    pub fn close(&self, resource_id: ResourceId) {
        self.commands.post(self.waker(), WakerCommand::Close(resource_id));
    }
}

/// Instance in charge of process input network events.
/// These events are offered to the user as a [`NetEvent`] its processing data.
pub struct NetworkProcessor {
    poll: Poll,
    command_receiver: EventReceiver<WakerCommand>,
    processors: EventProcessorList,
}

impl NetworkProcessor {
    fn new(
        poll: Poll,
        command_receiver: EventReceiver<WakerCommand>,
        processors: EventProcessorList,
    ) -> Self {
        Self { poll, command_receiver, processors }
    }

    /// Process the next poll event.
    /// This method waits the timeout specified until the poll event is generated.
    /// If `None` is passed as timeout, it will wait indefinitely.
    /// Note that there is no 1-1 relation between an internal poll event and a [`NetEvent`].
    /// You need to assume that process an internal poll event could call 0 or N times to
    /// the callback with diferents `NetEvent`s.
    pub fn process_poll_event(
        &mut self,
        timeout: Option<Duration>,
        event_callback: &mut dyn FnMut(NodeEvent),
    ) {
        let command_receiver = &mut self.command_receiver;
        let processors = &mut self.processors;
        self.poll.process_event(timeout, |poll_event| {
            match poll_event {
                PollEvent::NetworkRead(resource_id) => {
                    let processor = &mut processors[resource_id.adapter_id() as usize];
                    processor.process_read(resource_id, &mut |net_event| {
                        log::trace!("Processed {:?}", net_event);
                        event_callback(NodeEvent::Network(net_event));
                    });
                }

                PollEvent::NetworkWrite(resource_id) => {
                    let processor = &mut processors[resource_id.adapter_id() as usize];
                    processor.process_write(resource_id, &mut |net_event| {
                        log::trace!("Processed {:?}", net_event);
                        event_callback(NodeEvent::Network(net_event));
                    });
                }

                PollEvent::Waker => {
                    //
                    const MAX_COMMANDS: usize = 1024_usize;
                    let mut count = 0_usize;
                    while count < MAX_COMMANDS {
                        if let Some(command) = command_receiver.try_receive() {
                            //
                            match command {
                                WakerCommand::Greet(greet) => {
                                    //
                                    event_callback(NodeEvent::Waker(WakerCommand::Greet(greet)));
                                }
                                WakerCommand::Connect(config, addr, cb) => {
                                    let processor = &mut processors[config.id() as usize];
                                    let ret = processor.process_connect(config, addr);
                                    cb(ret);
                                }
                                WakerCommand::Listen(config, addr, cb) => {
                                    let processor = &mut processors[config.id() as usize];
                                    let ret = processor.process_listen(config, addr);
                                    cb(ret);
                                }
                                WakerCommand::Send(endpoint, buffer) => {
                                    let resource_id = endpoint.resource_id();
                                    let processor =
                                        &mut processors[resource_id.adapter_id() as usize];
                                    processor.process_send(endpoint, buffer);
                                }
                                WakerCommand::SendTrunk => {
                                    //
                                    event_callback(NodeEvent::Waker(WakerCommand::SendTrunk));
                                }
                                WakerCommand::Close(resource_id) => {
                                    //
                                    let processor =
                                        &mut processors[resource_id.adapter_id() as usize];
                                    processor.process_close(resource_id);
                                }
                                WakerCommand::Stop => {
                                    //
                                    event_callback(NodeEvent::Waker(WakerCommand::Stop));
                                }
                            }
                            count += 1;
                        } else {
                            //
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Process poll events until there is no more events during a `timeout` duration.
    /// This method makes succesive calls to [`NetworkProcessor::process_poll_event()`].
    pub fn process_poll_events_until_timeout(
        &mut self,
        timeout: Duration,
        event_callback: &mut dyn FnMut(NodeEvent),
    ) {
        let cb = &mut |e| event_callback(e);
        loop {
            let now = Instant::now();
            self.process_poll_event(Some(timeout), cb);
            if now.elapsed() > timeout {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use net_packet::take_packet;
    use test_case::test_case;

    use crate::network;
    use crate::node;
    use crate::util::thread::NamespacedThread;

    use super::*;

    lazy_static::lazy_static! {
        static ref TIMEOUT: Duration = Duration::from_millis(1000);
        static ref LOCALHOST_CONN_TIMEOUT: Duration = Duration::from_millis(5000);
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn successful_connection(transport: Transport) {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        //
        let handler2 = handler.clone();
        handler.network().listen(transport, "127.0.0.1:0", move |ret| {
            if let Ok((_listener_id, addr)) = ret {
                handler2.network().connect(transport, addr, |_| {});
            }
        });

        let mut was_connected = 0;
        let mut was_accepted = 0;
        processor.process_poll_events_until_timeout(*TIMEOUT, &mut |e| {
            let net_event = e.network();
            match net_event {
                NetEvent::Connected(_endpoint, status) => {
                    assert!(status);
                    was_connected += 1;
                }
                NetEvent::Accepted(_, _listener_id) => {
                    was_accepted += 1;
                }
                _ => unreachable!(),
            }
        });
        assert_eq!(was_accepted, 1);
        assert_eq!(was_connected, 1);
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn successful_connection_sync(transport: Transport) {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        //
        let mut thread = NamespacedThread::spawn("test", move || {
            //
            let (_listener_id, addr) =
                handler.network().listen_sync(transport, "127.0.0.1:0").unwrap();
            handler.network().connect_sync(transport, addr).unwrap();
        });

        processor.process_poll_events_until_timeout(*TIMEOUT, &mut |_| ());

        thread.join();
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn unreachable_connection(transport: Transport) {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        // Ensure that addr is not using by other process
        // because it takes some secs to be reusable.
        let handler2 = handler.clone();
        let transport2 = transport.clone();
        handler.network().listen(transport, "127.0.0.1:0", move |ret| {
            if let Ok((listener_id, addr)) = ret {
                handler2.network().close(listener_id);

                let handler3 = handler2.clone();
                handler2.network().connect(transport2, addr, move |ret| {
                    //
                    if let Ok((endpoint, _)) = ret {
                        let buffer = take_packet(42);
                        handler3.network().send(endpoint, buffer);
                    }
                });
            }
        });

        let mut was_disconnected = false;
        processor.process_poll_events_until_timeout(*LOCALHOST_CONN_TIMEOUT, &mut |e| {
            let net_event = e.network();
            match net_event {
                NetEvent::Connected(_endpoint, status) => {
                    assert!(!status);
                    was_disconnected = true;
                }
                _ => unreachable!(),
            }
        });
        assert!(was_disconnected);
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn unreachable_connection_sync(transport: Transport) {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        let mut thread = NamespacedThread::spawn("test", move || {
            // Ensure that addr is not using by other process
            // because it takes some secs to be reusable.
            let (listener_id, addr) =
                handler.network().listen_sync(transport, "127.0.0.1:0").unwrap();
            handler.network().close(listener_id);
            std::thread::sleep(std::time::Duration::from_secs(3));
            match handler.network().connect_sync(transport, addr) {
                Ok(_) => {
                    //
                    assert!(false);
                }
                Err(err) => {
                    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
                }
            };
        });

        processor.process_poll_events_until_timeout(*LOCALHOST_CONN_TIMEOUT, &mut |_| ());

        thread.join();
    }

    #[test]
    fn create_remove_listener() {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        let handler2 = handler.clone();
        handler.network().listen(Transport::Tcp, "127.0.0.1:0", move |ret| {
            if let Ok((listener_id, _addr)) = ret {
                handler2.network().close(listener_id); // Do not generate an event
                handler2.network().close(listener_id);
            }
        });

        processor.process_poll_events_until_timeout(*TIMEOUT, &mut |_| unreachable!());
    }

    #[test]
    fn create_remove_listener_with_connection() {
        let (engine, handler) = node::split();
        let mut processor = network::create_processor(engine);

        let handler2 = handler.clone();
        handler.network().listen(Transport::Tcp, "127.0.0.1:0", move |ret| {
            if let Ok((_listener_id, addr)) = ret {
                handler2.network().connect(Transport::Tcp, addr, |_| {});
            }
        });

        let mut was_accepted = false;
        processor.process_poll_events_until_timeout(*TIMEOUT, &mut |e| {
            let net_event = e.network();
            match net_event {
                NetEvent::Connected(..) => (),
                NetEvent::Accepted(_, listener_id) => {
                    handler.network().close(listener_id);
                    handler.network().close(listener_id);
                    was_accepted = true;
                }
                _ => unreachable!(),
            }
        });
        assert!(was_accepted);
    }
}
