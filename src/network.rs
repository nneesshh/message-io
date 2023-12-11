use std::sync::Arc;
use std::time::Duration;

use strum::IntoEnumIterator;

use crate::events::{EventReceiver, EventSender};
use crate::node::NodeHandler;
use crate::node_event::{NodeEvent, NodeEventEmitter};

///
pub mod net_event;

///
pub mod waker_command;

///
pub mod driver;

///
pub mod endpoint;

///
pub mod loader;

///
pub mod poll;

///
pub mod registry;

///
pub mod remote_addr;

///
pub mod resource_id;

///
pub mod transport;

/// Module that specify the pattern to follow to create adapters.
/// This module is not part of the public API itself,
/// it must be used from the internals to build new adapters.
pub mod adapter;

// Reexports
pub use adapter::SendStatus;
pub use driver::{ConnectConfig, ListenConfig};
pub use endpoint::Endpoint;
pub use loader::{Multiplexor, MultiplexorWorkerParam, MULTIPLEXOR_THREAD_NUM};
pub use net_event::NetEvent;
pub use poll::PollWaker;
pub use remote_addr::{RemoteAddr, ToRemoteAddr};
pub use resource_id::{ResourceId, ResourceIdGenerator, ResourceType};
pub use transport::Transport;
pub use waker_command::WakerCommand;

use loader::{BoxedEventProcessor, EventProcessorList, UnimplementedDriver};
use poll::{Poll, PollEvent};

/// Create a multiplexor worker instance.
pub(crate) fn create_multiplexor_worker(
    mut param: MultiplexorWorkerParam,
    node_handler: &NodeHandler,
    remote_id_generator: &Arc<ResourceIdGenerator>,
    local_id_generator: &Arc<ResourceIdGenerator>,
) -> MultiplexorWorker {
    let command_receiver = param.command_receiver().clone();
    let mut processors: EventProcessorList = (0..ResourceId::MAX_ADAPTERS)
        .map(|_| Box::new(UnimplementedDriver) as BoxedEventProcessor)
        .collect();
    Transport::iter().for_each(|transport| {
        transport.mount_adapter(
            &mut param,
            &node_handler,
            remote_id_generator,
            local_id_generator,
            &mut processors,
        )
    });
    MultiplexorWorker::new(param.take_poll(), command_receiver, processors)
}

/// Shareable instance in charge of control all the connections.
pub(crate) struct NetworkController {
    id: usize,
    commands: EventSender<WakerCommand>,
    waker: PollWaker,
}

impl NetworkController {
    ///
    pub(crate) fn new(
        id: usize,
        commands: EventSender<WakerCommand>,
        waker: PollWaker,
    ) -> NetworkController {
        Self { id, commands, waker }
    }

    ///
    #[inline(always)]
    #[allow(dead_code)]
    pub fn id(&self) -> usize {
        self.id
    }

    /// post waker command to worker queue
    #[inline(always)]
    pub(crate) fn do_post(&self, command: WakerCommand) {
        //log::info!("do_post controller({}) command:{:?}", self.id, command);
        self.commands.post(&self.waker, command);
    }
}

/// Instance in charge of process input network events.
/// These events are offered to the user as a [`NetEvent`] its processing data.
pub struct MultiplexorWorker {
    poll: Poll,
    command_receiver: EventReceiver<WakerCommand>,
    processors: EventProcessorList,
}

impl MultiplexorWorker {
    fn new(
        poll: Poll,
        command_receiver: EventReceiver<WakerCommand>,
        processors: EventProcessorList,
    ) -> Self {
        Self { poll, command_receiver, processors }
    }

    ///
    #[inline(always)]
    pub fn event_processor(&mut self, adapter_id: u8) -> &mut BoxedEventProcessor {
        get_processor(&mut self.processors, adapter_id)
    }

    /// Process the next poll event.
    /// This method waits the timeout specified until the poll event is generated.
    /// If `None` is passed as timeout, it will wait indefinitely.
    /// Note that there is no 1-1 relation between an internal poll event and a [`NetEvent`].
    /// You need to assume that process an internal poll event could call 0 or N times to
    /// the callback with diferents `NetEvent`s.
    pub fn process_poll_event<E>(&mut self, timeout: Option<Duration>, emitter: &mut E)
    where
        E: NodeEventEmitter + 'static,
    {
        let command_receiver = &mut self.command_receiver;
        let processors = &mut self.processors;
        let poll = &mut self.poll;

        poll.process_event(timeout, |poll_event| {
            match poll_event {
                PollEvent::NetworkRead(resource_id) => {
                    let adapter_id = resource_id.adapter_id();

                    //
                    let processor = get_processor(processors, adapter_id);
                    processor.process_read(resource_id, &mut |net_event| {
                        log::trace!("Processed {:?}", net_event);
                        emitter.emit(NodeEvent::Network(net_event));
                    });
                }

                PollEvent::NetworkWrite(resource_id) => {
                    let adapter_id = resource_id.adapter_id();

                    //
                    let processor = get_processor(processors, adapter_id);
                    processor.process_write(resource_id, &mut |net_event| {
                        log::trace!("Processed {:?}", net_event);
                        emitter.emit(NodeEvent::Network(net_event));
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
                                    emitter.emit(NodeEvent::Waker(WakerCommand::Greet(greet)));
                                }
                                WakerCommand::AcceptRegisterRemote(
                                    local_pair,
                                    remote_pair,
                                    stream,
                                    payload,
                                ) => {
                                    let remote_id = remote_pair.0;
                                    let adapter_id = remote_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_accept_register_remote(
                                        local_pair,
                                        remote_pair,
                                        stream,
                                        payload,
                                        Box::new(|_h, _ret| {
                                            //
                                            //log::trace!("accept register_remote {_ret:?}");
                                        }),
                                    );
                                }
                                WakerCommand::ConnectRegisterRemote(
                                    local_addr,
                                    remote_pair,
                                    stream,
                                    keepalive_opt,
                                    domain_opt,
                                    cb,
                                ) => {
                                    let remote_id = remote_pair.0;
                                    let adapter_id = remote_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_connect_register_remote(
                                        local_addr,
                                        remote_pair,
                                        stream,
                                        keepalive_opt,
                                        domain_opt,
                                        cb,
                                    );
                                }
                                WakerCommand::Listen(config, local_id, addr, cb) => {
                                    let adapter_id = local_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_listen(config, local_id, addr, cb);
                                }
                                WakerCommand::Connect(config, remote_id, raddr, cb) => {
                                    let adapter_id = remote_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_connect(config, remote_id, raddr, cb);
                                }
                                WakerCommand::Send(endpoint, buffer) => {
                                    let resource_id = endpoint.resource_id();
                                    let adapter_id = resource_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_send(endpoint, buffer.peek());
                                }
                                WakerCommand::SendSync(endpoint, buffer, cb) => {
                                    let resource_id = endpoint.resource_id();
                                    let adapter_id = resource_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_send(endpoint, buffer.peek());
                                    cb();
                                }
                                WakerCommand::Close(resource_id) => {
                                    let adapter_id = resource_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_close(
                                        resource_id,
                                        Box::new(move |_h, _ret| {
                                            //
                                            //log::trace!("close {resource_id} {_ret:?}");
                                        }),
                                    );
                                }
                                WakerCommand::IsReady(resource_id, cb) => {
                                    let adapter_id = resource_id.adapter_id();

                                    //
                                    let processor = get_processor(processors, adapter_id);
                                    processor.process_is_ready(resource_id, cb);
                                }
                                WakerCommand::Stop => {
                                    //
                                    emitter.emit(NodeEvent::Waker(WakerCommand::Stop));
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
}

#[inline(always)]
fn get_processor(processors: &mut EventProcessorList, adapter_id: u8) -> &mut BoxedEventProcessor {
    &mut processors[adapter_id as usize]
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use net_packet::take_packet;
    use parking_lot::Mutex;
    use test_case::test_case;

    use crate::node;
    use crate::util::thread::NamespacedThread;

    use super::*;

    lazy_static::lazy_static! {
        static ref TIMEOUT: Duration = Duration::from_millis(5000);
        static ref LOCALHOST_CONN_TIMEOUT: Duration = Duration::from_millis(10000);
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn successful_connection(transport: Transport) {
        let (mux, handler) = node::split();

        //
        handler.listen(transport, "127.0.0.1:0", move |h, ret| {
            if let Ok((_listener_id, addr)) = ret {
                h.connect(transport, addr, |h, _| {
                    //
                    h.stop();
                });
            }
        });

        let mut task = node::node_listener_for_each_async(mux, &handler, move |_h, _| {});
        task.wait();
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn successful_connection_sync(transport: Transport) {
        let (mux, handler) = node::split();

        //
        let handler2 = handler.clone();
        let mut thread = NamespacedThread::spawn("test", move || {
            //
            let (_listener_id, addr) = handler2.listen_sync(transport, "127.0.0.1:0").unwrap();
            let _ = handler2.connect_sync(transport, addr).unwrap();
        });

        node::node_listener_for_each_async(mux, &handler, move |_h, _| {});

        thread.join();
        handler.stop();
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn unreachable_connection(transport: Transport) {
        let (mux, handler) = node::split();

        // Ensure that addr is not using by other process
        // because it takes some secs to be reusable.
        let transport2 = transport.clone();
        handler.listen(transport, "127.0.0.1:0", move |h, ret| {
            if let Ok((listener_id, addr)) = ret {
                h.close(listener_id);

                h.connect(transport2, addr, move |h, ret| {
                    //
                    if let Ok((endpoint, _)) = ret {
                        let buffer = take_packet(42);
                        h.send(endpoint, buffer);
                    }
                });
            }
        });

        let was_disconnected = Arc::new(Mutex::new(false));
        let was_disconnected2 = was_disconnected.clone();
        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, e| {
            let net_event = e.network();
            match net_event {
                NetEvent::Connected(_endpoint, status) => {
                    assert!(!status);

                    let mut guard = was_disconnected2.lock();
                    *guard = true;
                    h.stop();
                }
                _ => {}
            }
        });
        task.wait();

        let guard = was_disconnected.lock();
        assert!(*guard);
    }

    #[cfg_attr(feature = "tcp", test_case(Transport::Tcp))]
    fn unreachable_connection_sync(transport: Transport) {
        let (mux, handler) = node::split();

        let handler2 = handler.clone();
        let mut thread = NamespacedThread::spawn("test", move || {
            // Ensure that addr is not using by other process
            // because it takes some secs to be reusable.
            let (listener_id, addr) = handler2.listen_sync(transport, "127.0.0.1:0").unwrap();
            handler2.close(listener_id);
            std::thread::sleep(std::time::Duration::from_secs(1));
            match handler2.connect_sync(transport, addr) {
                Ok(_) => {
                    //
                    assert!(false);
                }
                Err(err) => {
                    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
                }
            };
        });

        node::node_listener_for_each_async(mux, &handler, move |_h, _| {});

        thread.join();
        handler.stop();
    }

    #[test]
    fn create_remove_listener() {
        let (mux, handler) = node::split();

        handler.listen(Transport::Tcp, "127.0.0.1:0", move |h, ret| {
            if let Ok((listener_id, _addr)) = ret {
                h.close(listener_id); // Do not generate an event
                h.close(listener_id);
                h.stop();
            }
        });

        let mut task = node::node_listener_for_each_async(mux, &handler, move |_h, _| {});
        task.wait();
    }

    #[test]
    fn create_remove_listener_with_connection() {
        let (mux, handler) = node::split();

        handler.listen(Transport::Tcp, "127.0.0.1:0", move |h, ret| {
            if let Ok((_listener_id, addr)) = ret {
                h.connect(Transport::Tcp, addr, |_h, _| {});
            }
        });

        let was_accepted = Arc::new(Mutex::new(false));
        let was_accepted2 = was_accepted.clone();
        let mut task = node::node_listener_for_each_async(mux, &handler, move |h, e| {
            let net_event = e.network();
            match net_event {
                NetEvent::Connected(..) => {
                    //
                    println!("Connected");
                }
                NetEvent::Accepted(_, listener_id) => {
                    h.close(listener_id);
                    h.close(listener_id);
                    let mut guard = was_accepted2.lock();
                    *guard = true;
                    h.stop();
                }
                _ => {}
            }
        });
        task.wait();

        let guard = was_accepted.lock();
        assert!(*guard);
    }
}
