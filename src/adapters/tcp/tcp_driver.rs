use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;

use mio::net::TcpStream;
use net_packet::take_small_packet;
use socket2::TcpKeepalive;

use crate::adapters::tcp::tcp_remote::tcp_remote_connect_with;
use crate::network::adapter::{AcceptedType, Local, PendingStatus, ReadStatus, Remote};
use crate::network::driver::{
    ConnectConfig, EventProcessor, ListenConfig, LocalProperties, RemoteProperties,
};
use crate::network::poll::Poll;
use crate::network::registry::{LocklessResourceRegistry, Register};
use crate::network::resource_id::{ResourceId, ResourceIdGenerator, ResourceType};
use crate::network::{Endpoint, NetEvent, RemoteAddr, SendStatus, WakerCommand};
use crate::node::NodeHandler;
use crate::util::unsafe_any::{UnsafeAny, UnsafeAnyExt};

use super::tcp_adapter::{LocalResource, RemoteResource, TcpAcceptPayload};

/// Poll registry for one adapter
pub struct TcpDriver {
    remote_id_generator: Arc<ResourceIdGenerator>,

    node_handler: NodeHandler,
    remote_registry: LocklessResourceRegistry<RemoteResource, RemoteProperties>,
    local_registry: LocklessResourceRegistry<LocalResource, LocalProperties>,
}

impl TcpDriver {
    ///
    pub fn new(
        rgen: &Arc<ResourceIdGenerator>,
        node_handler: NodeHandler,
        poll: &mut Poll,
    ) -> Self {
        let remote_poll_registry = poll.create_registry();
        let local_poll_registry = poll.create_registry();

        Self {
            remote_id_generator: rgen.clone(),

            node_handler,
            remote_registry: LocklessResourceRegistry::<RemoteResource, RemoteProperties>::new(
                remote_poll_registry,
            ),
            local_registry: LocklessResourceRegistry::<LocalResource, LocalProperties>::new(
                local_poll_registry,
            ),
        }
    }

    #[inline(always)]
    fn next_remote_id(&self, adapter_id: u8) -> ResourceId {
        self.remote_id_generator.generate(adapter_id)
    }

    #[inline(always)]
    fn resolve_pending_remote(
        &self,
        remote: &mut Rc<Register<RemoteResource, RemoteProperties>>,
        endpoint: Endpoint,
        mut event_callback: impl FnMut(NetEvent),
    ) {
        let status = remote.resource.pending();
        log::trace!("Resolve pending for {}: {:?}", endpoint, status);
        match status {
            PendingStatus::Ready => {
                remote.properties.mark_as_ready();
                match remote.properties.local {
                    Some(listener_id) => event_callback(NetEvent::Accepted(endpoint, listener_id)),
                    None => event_callback(NetEvent::Connected(endpoint, true)),
                }
                remote.resource.ready_to_write();
            }
            PendingStatus::Incomplete => (),
            PendingStatus::Disconnected => {
                self.remote_registry.deregister(endpoint.resource_id());
                if remote.properties.local.is_none() {
                    event_callback(NetEvent::Connected(endpoint, false));
                }
            }
        }
    }

    #[inline(always)]
    fn write_to_remote(
        &self,
        remote: &mut Rc<Register<RemoteResource, RemoteProperties>>,
        endpoint: Endpoint,
        mut event_callback: impl FnMut(NetEvent),
    ) {
        if !remote.resource.ready_to_write() {
            event_callback(NetEvent::Disconnected(endpoint));
        }
    }

    #[inline(always)]
    fn read_from_remote(
        &self,
        remote: &mut Rc<Register<RemoteResource, RemoteProperties>>,
        endpoint: Endpoint,
        mut event_callback: impl FnMut(NetEvent),
    ) {
        let status =
            remote.resource.receive(&mut |data| event_callback(NetEvent::Message(endpoint, data)));
        log::trace!("Receive status: {:?}", status);
        if let ReadStatus::Disconnected = status {
            // Checked because, the user in the callback could have removed the same resource.
            if self.remote_registry.deregister(endpoint.resource_id()) {
                event_callback(NetEvent::Disconnected(endpoint));
            }
        }
    }

    #[inline(always)]
    fn read_from_local(
        &self,
        handler: &NodeHandler,
        local: &mut Rc<Register<LocalResource, LocalProperties>>,
        id: ResourceId,
        mut event_callback: impl FnMut(NetEvent),
    ) {
        local.resource.accept(|accepted| {
            //log::trace!("Accepted type: {}", accepted);
            match accepted {
                AcceptedType::Remote(remote_addr, stream, payload) => {
                    //
                    let local_addr = local.resource.listener.local_addr().unwrap();

                    //
                    let adapter_id = id.adapter_id();
                    let remote_id = self.next_remote_id(adapter_id);

                    // register TcpStream
                    handler.post(
                        remote_id,
                        WakerCommand::AcceptRegisterRemote(
                            (id, local_addr),
                            (remote_id, remote_addr),
                            stream,
                            payload,
                        ),
                    );
                }
                AcceptedType::Data(addr, data) => {
                    let endpoint = Endpoint::new(id, addr);

                    let mut input_buffer = take_small_packet();
                    input_buffer.append_slice(data);
                    event_callback(NetEvent::Message(endpoint, input_buffer));
                }
            }
        });
    }
}

impl Clone for TcpDriver {
    fn clone(&self) -> Self {
        Self {
            remote_id_generator: self.remote_id_generator.clone(),

            node_handler: self.node_handler.clone(),
            remote_registry: self.remote_registry.clone(),
            local_registry: self.local_registry.clone(),
        }
    }
}

impl EventProcessor for TcpDriver {
    #[inline(always)]
    fn process_read(&mut self, id: ResourceId, event_callback: &mut dyn FnMut(NetEvent)) {
        match id.resource_type() {
            ResourceType::Remote => {
                if let Some(ref mut remote) = self.remote_registry.get(id) {
                    let endpoint = Endpoint::new(id, remote.properties.peer_addr);
                    //log::trace!("Processed remote for {}", endpoint);

                    //
                    if !remote.properties.is_ready() {
                        self.resolve_pending_remote(remote, endpoint, |e| {
                            //
                            event_callback(e);
                        });
                    }

                    //
                    if remote.properties.is_ready() {
                        //
                        self.read_from_remote(remote, endpoint, event_callback);
                    }
                }
            }
            ResourceType::Local => {
                if let Some(ref mut local) = self.local_registry.get(id) {
                    log::trace!("Processed local for {}", id);

                    //
                    self.read_from_local(&self.node_handler, local, id, event_callback)
                }
            }
        }
    }

    #[inline(always)]
    fn process_write(&mut self, id: ResourceId, event_callback: &mut dyn FnMut(NetEvent)) {
        match id.resource_type() {
            ResourceType::Remote => {
                if let Some(ref mut remote) = self.remote_registry.get(id) {
                    let endpoint = Endpoint::new(id, remote.properties.peer_addr);
                    //log::trace!("Processed remote for {}", endpoint);

                    //
                    if !remote.properties.is_ready() {
                        self.resolve_pending_remote(remote, endpoint, |e| {
                            //
                            event_callback(e);
                        });
                    }

                    //
                    if remote.properties.is_ready() {
                        //
                        self.write_to_remote(remote, endpoint, event_callback);
                    }
                }
            }
            ResourceType::Local => {
                // do nothing
            }
        }
    }

    fn process_accept_register_remote(
        &mut self,
        local_pair: (ResourceId, SocketAddr),
        remote_pair: (ResourceId, SocketAddr),
        stream: TcpStream,
        payload: Box<dyn UnsafeAny + Send>,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        let local_id = local_pair.0;
        let remote_id = remote_pair.0;
        let remote_addr = remote_pair.1;

        // register remote resource
        let payload = unsafe {
            //
            *payload.downcast_unchecked::<TcpAcceptPayload>()
        };

        let resource = RemoteResource { stream, keepalive_opt: payload.keepalive_opt };
        self.remote_registry.register(
            remote_id,
            resource,
            RemoteProperties::new(remote_addr, Some(local_id)),
            true,
        );

        // callback
        let endpoint = Endpoint::new(remote_id, remote_addr);
        let local_addr = local_pair.1;
        callback(&self.node_handler, Ok((endpoint, local_addr)));
    }

    fn process_connect_register_remote(
        &mut self,
        local_addr: SocketAddr,
        remote_pair: (ResourceId, SocketAddr),
        stream: TcpStream,
        keepalive_opt: Option<TcpKeepalive>,
        _domain_opt: Option<String>,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        let remote_id = remote_pair.0;
        let remote_addr = remote_pair.1;

        // register remote resource
        let resource = RemoteResource { stream, keepalive_opt };
        self.remote_registry.register(
            remote_id,
            resource,
            RemoteProperties::new(remote_addr, None),
            true,
        );

        // callback
        let endpoint = Endpoint::new(remote_id, remote_addr);
        callback(&self.node_handler, Ok((endpoint, local_addr)));
    }

    fn process_listen(
        &mut self,
        config: ListenConfig,
        local_id: ResourceId,
        addr: SocketAddr,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(ResourceId, SocketAddr)>) + Send>,
    ) {
        //
        let ret = LocalResource::listen_with(config, addr)
            .map(|info| {
                self.local_registry.register(local_id, info.local, LocalProperties, false);
                (local_id, info.local_addr)
            })
            .map(|(resource_id, addr)| {
                log::trace!("Listening at {} by {}", addr, resource_id);
                (resource_id, addr)
            });
        callback(&self.node_handler, ret);
    }

    fn process_connect(
        &mut self,
        config: ConnectConfig,
        remote_id: ResourceId,
        raddr: RemoteAddr,
        callback: Box<dyn FnOnce(&NodeHandler, io::Result<(Endpoint, SocketAddr)>) + Send>,
    ) {
        //
        let peer_addr = *raddr.socket_addr();

        //
        let connection_info_ret = tcp_remote_connect_with(config, peer_addr);
        let _ = connection_info_ret.map(|info| {
            // register TcpStream
            self.node_handler.post(
                remote_id,
                WakerCommand::ConnectRegisterRemote(
                    info.local_addr,
                    (remote_id, info.peer_addr),
                    info.stream,
                    info.keepalive_opt,
                    None,
                    callback,
                ),
            );
        });
    }

    #[inline(always)]
    fn process_send(&mut self, endpoint: Endpoint, data: &[u8]) {
        let id = endpoint.resource_id();
        let _status = match id.resource_type() {
            ResourceType::Remote => {
                //
                match self.remote_registry.get(id) {
                    Some(ref mut remote) => {
                        //
                        if remote.properties.is_ready() {
                            remote.resource.send(data)
                        } else {
                            SendStatus::ResourceNotAvailable
                        }
                    }
                    None => SendStatus::ResourceNotFound,
                }
            }
            ResourceType::Local => {
                //
                match self.local_registry.get(id) {
                    Some(remote) => remote.resource.send_to(endpoint.addr(), data),
                    None => SendStatus::ResourceNotFound,
                }
            }
        };
    }

    #[inline(always)]
    fn process_close(
        &mut self,
        id: ResourceId,
        callback: Box<dyn FnOnce(&NodeHandler, bool) + Send>,
    ) {
        let ret = match id.resource_type() {
            ResourceType::Remote => self.remote_registry.deregister(id),
            ResourceType::Local => self.local_registry.deregister(id),
        };
        callback(&self.node_handler, ret);
    }

    #[inline(always)]
    fn process_is_ready(
        &mut self,
        id: ResourceId,
        callback: Box<dyn FnOnce(&NodeHandler, Option<bool>) + Send>,
    ) {
        match id.resource_type() {
            ResourceType::Remote => {
                //
                let r_opt = self.remote_registry.get(id);
                match r_opt {
                    Some(r) => {
                        //
                        callback(&self.node_handler, Some(r.properties.is_ready()));
                    }
                    None => {
                        //
                        callback(&self.node_handler, None);
                    }
                }
            }
            ResourceType::Local => {
                //
                let l_opt = self.remote_registry.get(id);
                callback(&self.node_handler, l_opt.map(|_| true));
            }
        }
    }
}
