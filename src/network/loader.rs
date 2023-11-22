use net_packet::NetPacketGuard;

use crate::network::{TransportConnect, TransportListen};

use super::SendStatus;
use super::adapter::Adapter;
use super::driver::{Driver, EventProcessor, NetEvent};
use super::endpoint::Endpoint;
use super::poll::Poll;
use super::remote_addr::RemoteAddr;
use super::resource_id::ResourceId;

use std::io::{self};
use std::net::SocketAddr;

type Processor = Box<dyn EventProcessor + Send>;

pub type EventProcessorList = Vec<Processor>;

/// Used to configured the engine
pub struct DriverLoader {
    poll: Poll,
    processors: EventProcessorList,
}

impl Default for DriverLoader {
    fn default() -> DriverLoader {
        Self {
            poll: Poll::default(),
            processors: (0..ResourceId::MAX_ADAPTERS)
                .map(|_| Box::new(UnimplementedDriver) as Processor)
                .collect(),
        }
    }
}

impl DriverLoader {
    /// Mount an adapter to create its driver associating it with an id.
    pub fn mount(&mut self, adapter_id: u8, adapter: impl Adapter + 'static) {
        let index = adapter_id as usize;

        let driver = Driver::new(adapter, adapter_id, &mut self.poll);
        self.processors[index] = Box::new(driver) as Processor;
    }

    /// Consume this instance to obtain the driver handles.
    pub fn take(self) -> (Poll, EventProcessorList) {
        (self.poll, self.processors)
    }
}

// The following unimplemented driver is used to fill
// the invalid adapter id gaps in the controllers/processors lists.
// It is faster and cleanest than to use an option that always must to be unwrapped.

const UNIMPLEMENTED_DRIVER_ERR: &str =
    "The chosen adapter id doesn't reference an existing adapter";

struct UnimplementedDriver;

impl EventProcessor for UnimplementedDriver {
    fn process_read(&mut self, _: ResourceId, _: &mut dyn FnMut(NetEvent)) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_write(&mut self, _: ResourceId, _: &mut dyn FnMut(NetEvent)) {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_connect(&mut self, _: TransportConnect, _: RemoteAddr) -> io::Result<(Endpoint, SocketAddr)> {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_listen(&mut self, _: TransportListen, _: SocketAddr) -> io::Result<(ResourceId, SocketAddr)> {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_send(&mut self, _: Endpoint, _: NetPacketGuard) -> SendStatus {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
    fn process_close(&mut self, _: ResourceId,) -> bool {
        panic!("{}", UNIMPLEMENTED_DRIVER_ERR);
    }
}
