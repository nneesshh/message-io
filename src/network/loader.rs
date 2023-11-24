use crate::events::{self, EventReceiver, EventSender};
use crate::WakerCommand;

use super::adapter::Adapter;
use super::driver::{Driver, EventProcessor, NetEvent};
use super::endpoint::Endpoint;
use super::poll::Poll;
use super::remote_addr::RemoteAddr;
use super::resource_id::ResourceId;
use super::SendStatus;
use super::{TransportConnect, TransportListen};

use std::io::{self};
use std::net::SocketAddr;

///
pub type BoxedEventProcessor = Box<dyn EventProcessor>;

///
pub type EventProcessorList = Vec<BoxedEventProcessor>;

/// Used to configured the engine
pub struct PollEngine {
    poll: Poll,
    command_sender: EventSender<WakerCommand>,
    command_receiver: EventReceiver<WakerCommand>,
}

impl Default for PollEngine {
    fn default() -> PollEngine {
        let (command_sender, command_receiver) = events::split();
        Self { poll: Poll::default(), command_sender, command_receiver }
    }
}

impl PollEngine {
    ///
    #[inline(always)]
    pub fn poll(&self) -> &Poll {
        &self.poll
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

    /// Mount an adapter to create its driver associating it with an id.
    pub fn mount(
        &mut self,
        adapter_id: u8,
        adapter: impl Adapter + 'static,
        processors: &mut EventProcessorList,
    ) {
        let poll = &mut self.poll;
        let index = adapter_id as usize;

        let driver = Driver::new(adapter, adapter_id, poll);
        processors[index] = Box::new(driver) as BoxedEventProcessor;
    }

    /// Consume this instance to obtain the poll handles.
    pub fn take(self) -> Poll {
        self.poll
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
    fn process_connect(
        &mut self,
        _: TransportConnect,
        _: RemoteAddr,
    ) -> io::Result<(Endpoint, SocketAddr)> {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_listen(
        &mut self,
        _: TransportListen,
        _: SocketAddr,
    ) -> io::Result<(ResourceId, SocketAddr)> {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_send(&mut self, _: Endpoint, _: &[u8]) -> SendStatus {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_close(&mut self, _: ResourceId) -> bool {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_is_ready(&mut self, _: ResourceId) -> Option<bool> {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
}
