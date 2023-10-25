#![allow(unused_variables)]

use net_packet::NetPacketGuard;

use crate::network::adapter::{
    AcceptedType, Adapter, ConnectionInfo, ListeningInfo, Local, PendingStatus, ReadStatus, Remote,
    Resource, SendStatus,
};
use crate::network::{Readiness, RemoteAddr, TransportConnect, TransportListen};

use mio::event::Source;

use std::io::{self};
use std::net::SocketAddr;

pub(crate) struct MyAdapter;
impl Adapter for MyAdapter {
    type Remote = RemoteResource;
    type Local = LocalResource;
}

pub(crate) struct RemoteResource;
impl Resource for RemoteResource {
    fn source(&mut self) -> &mut dyn Source {
        todo!()
    }
}

impl Remote for RemoteResource {
    fn connect_with(
        config: TransportConnect,
        remote_addr: RemoteAddr,
    ) -> io::Result<ConnectionInfo<Self>> {
        todo!()
    }

    fn receive(&self, process_data: impl FnMut(NetPacketGuard)) -> ReadStatus {
        todo!()
    }

    fn send(&self, data: &[u8]) -> SendStatus {
        todo!()
    }

    fn pending(&self, _readiness: Readiness) -> PendingStatus {
        todo!()
    }
}

pub(crate) struct LocalResource;
impl Resource for LocalResource {
    fn source(&mut self) -> &mut dyn Source {
        todo!()
    }
}

impl Local for LocalResource {
    type Remote = RemoteResource;

    fn listen_with(config: TransportListen, addr: SocketAddr) -> io::Result<ListeningInfo<Self>> {
        todo!()
    }

    fn accept(&self, accept_remote: impl FnMut(AcceptedType<'_, Self::Remote>)) {
        todo!()
    }
}
