use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use mio::net::TcpStream;

use crate::events::{self, EventReceiver, EventSender};
use crate::node::NodeHandler;
use crate::util::unsafe_any::UnsafeAny;

use super::driver::{ConnectConfig, EventProcessor, ListenConfig};
use super::endpoint::Endpoint;
use super::net_event::NetEvent;
use super::poll::Poll;
use super::resource_id::ResourceId;
use super::waker_command::WakerCommand;
use super::{RemoteAddr, ResourceIdGenerator, ResourceType};

///
pub type BoxedEventProcessor = Box<dyn EventProcessor>;

///
pub type EventProcessorList = Vec<BoxedEventProcessor>;

/// Used to configure the event processor
pub struct MultiplexorWorkerParam {
    poll_opt: Option<Poll>,
    command_sender: EventSender<WakerCommand>,
    command_receiver: EventReceiver<WakerCommand>,
}

impl Default for MultiplexorWorkerParam {
    fn default() -> MultiplexorWorkerParam {
        let (command_sender, command_receiver) = events::split();
        Self { poll_opt: Some(Poll::default()), command_sender, command_receiver }
    }
}

impl MultiplexorWorkerParam {
    ///
    #[inline(always)]
    pub fn poll(&mut self) -> &mut Poll {
        self.poll_opt.as_mut().unwrap()
    }

    /// Consume this instance to obtain the poll handles.
    pub fn take_poll(mut self) -> Poll {
        self.poll_opt.take().unwrap()
    }

    ///
    #[inline(always)]
    pub fn command_sender(&self) -> &EventSender<WakerCommand> {
        &self.command_sender
    }

    ///
    #[inline(always)]
    pub fn command_receiver(&self) -> &EventReceiver<WakerCommand> {
        &self.command_receiver
    }
}

fn split_id_generator() -> (Arc<ResourceIdGenerator>, Arc<ResourceIdGenerator>) {
    let rgen = Arc::new(ResourceIdGenerator::new(ResourceType::Remote));
    let lgen = Arc::new(ResourceIdGenerator::new(ResourceType::Local));
    (rgen, lgen)
}

///
pub struct Multiplexor {
    pub rgen: Arc<ResourceIdGenerator>,
    pub lgen: Arc<ResourceIdGenerator>,
    pub master_param: Option<MultiplexorWorkerParam>,
    pub worker_params: Vec<Option<MultiplexorWorkerParam>>,
}

impl Default for Multiplexor {
    fn default() -> Multiplexor {
        let (rgen, lgen) = split_id_generator();
        Self {
            //
            rgen,
            lgen,
            master_param: None,
            worker_params: Vec::new(),
        }
    }
}

// The following unimplemented driver is used to fill
// the invalid adapter id gaps in the controllers/processors lists.
// It is faster and cleanest than to use an option that always must to be unwrapped.

const UNIMPLEMENTED_DRIVER_ERR: &str =
    "The chosen adapter id doesn't reference an existing adapter";

///
pub struct UnimplementedDriver;

impl EventProcessor for UnimplementedDriver {
    fn process_read(&mut self, _: ResourceId, _: &mut dyn FnMut(NetEvent)) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_write(&mut self, _: ResourceId, _: &mut dyn FnMut(NetEvent)) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_accept_register_remote(
        &mut self,
        _: (ResourceId, SocketAddr),
        _: (ResourceId, SocketAddr),
        _: TcpStream,
        _: Box<dyn UnsafeAny + Send>,
        _: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_connect_register_remote(
        &mut self,
        _: SocketAddr,
        _: (ResourceId, SocketAddr),
        _: TcpStream,
        _: Box<dyn UnsafeAny + Send>,
        _: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_listen(
        &mut self,
        _: ListenConfig,
        _: ResourceId,
        _: SocketAddr,
        _: Box<dyn FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send>,
    ) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_connect(
        &mut self,
        _: ConnectConfig,
        _: ResourceId,
        _: RemoteAddr,
        _: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_send(&mut self, _: Endpoint, _: &[u8]) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_close(&mut self, _: ResourceId, _: Box<dyn FnOnce(&NodeHandler, bool) + Send>) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_is_ready(
        &mut self,
        _: ResourceId,
        _: Box<dyn FnOnce(&NodeHandler, Option<bool>) + Send>,
    ) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
}
