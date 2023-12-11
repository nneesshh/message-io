use super::resource_id::{ResourceId, RESERVED_BYTES_MASK, RESERVED_BYTES_POS};

use mio::event::Source;
use mio::{Events, Interest, Poll as MioPoll, Registry, Token, Waker};

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

pub enum PollEvent {
    NetworkRead(ResourceId),
    NetworkWrite(ResourceId),
    Waker,
}

impl From<Token> for ResourceId {
    fn from(token: Token) -> Self {
        (token.0 & !RESERVED_BYTES_MASK).into()
    }
}

impl From<ResourceId> for Token {
    fn from(id: ResourceId) -> Self {
        Token((id.raw() & !RESERVED_BYTES_MASK) | (0x55 << RESERVED_BYTES_POS))
    }
}

pub struct Poll {
    mio_poll: MioPoll,
    events: Events,
    waker: Arc<Waker>,
}

impl Default for Poll {
    fn default() -> Self {
        let mio_poll = MioPoll::new().unwrap();
        Self {
            waker: Arc::new(Waker::new(mio_poll.registry(), Self::WAKER_TOKEN).unwrap()),
            mio_poll,
            events: Events::with_capacity(Self::EVENTS_SIZE),
        }
    }
}

impl Poll {
    const EVENTS_SIZE: usize = 1024;
    const WAKER_TOKEN: Token = Token(0);

    ///
    #[inline(always)]
    pub fn process_event<C>(&mut self, timeout: Option<Duration>, mut event_callback: C)
    where
        C: FnMut(PollEvent),
    {
        loop {
            match self.mio_poll.poll(&mut self.events, timeout) {
                Ok(()) => {
                    for mio_event in &self.events {
                        if Self::WAKER_TOKEN == mio_event.token() {
                            log::trace!("POLL WAKER EVENT");
                            event_callback(PollEvent::Waker);
                        } else {
                            let id = ResourceId::from(mio_event.token());
                            if mio_event.is_readable() {
                                log::trace!("POLL EVENT (R): {}", id);
                                event_callback(PollEvent::NetworkRead(id));
                            }
                            if mio_event.is_writable() {
                                log::trace!("POLL EVENT (W): {}", id);
                                event_callback(PollEvent::NetworkWrite(id));
                            }
                        }
                    }
                    break;
                }
                Err(ref err) if err.kind() == ErrorKind::Interrupted => {
                    //
                    continue;
                }
                Err(ref err) => {
                    //
                    Err(err).expect("No error here")
                }
            }
        }
    }

    ///
    #[inline(always)]
    pub fn create_registry(&mut self) -> PollRegistry {
        PollRegistry::new(self.mio_poll.registry().try_clone().unwrap())
    }

    ///
    #[inline(always)]
    pub fn create_waker(&self) -> PollWaker {
        PollWaker::new(self.waker.clone())
    }
}

pub struct PollRegistry {
    registry: Registry,
}

impl PollRegistry {
    fn new(registry: Registry) -> Self {
        Self { registry }
    }

    ///
    #[inline(always)]
    pub fn add(
        &self,
        id: ResourceId,
        source: &mut dyn Source,
        write_readiness: bool,
    ) -> ResourceId {
        let interest = match write_readiness {
            true => Interest::READABLE | Interest::WRITABLE,
            false => Interest::READABLE,
        };
        self.registry.register(source, id.into(), interest).unwrap();
        id
    }

    ///
    #[inline(always)]
    pub fn remove(&self, source: &mut dyn Source) {
        self.registry.deregister(source).unwrap()
    }
}

impl Clone for PollRegistry {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self { registry: self.registry.try_clone().unwrap() }
    }
}

///
pub struct PollWaker {
    waker: Arc<Waker>,
}

impl PollWaker {
    #[inline(always)]
    fn new(waker: Arc<Waker>) -> Self {
        Self { waker }
    }

    ///
    #[inline(always)]
    pub fn wake(&self) {
        self.waker.wake().unwrap();
        log::trace!("Wake poll...");
    }
}

impl Clone for PollWaker {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self { waker: self.waker.clone() }
    }
}
