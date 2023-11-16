use hashbrown::HashMap;
use parking_lot::RwLock;
use std::sync::Arc;

use super::adapter::Resource;
use super::poll::PollRegistry;
use super::resource_id::ResourceId;

/// Register of resource
pub struct Register<S: Resource, P> {
    pub resource: S,
    pub properties: P,

    poll_registry: Arc<PollRegistry>,
}

impl<S: Resource, P> Register<S, P> {
    fn new(resource: S, properties: P, poll_registry: Arc<PollRegistry>) -> Self {
        Self { resource, properties, poll_registry }
    }
}

impl<S: Resource, P> Drop for Register<S, P> {
    fn drop(&mut self) {
        self.poll_registry.remove(self.resource.source());
    }
}

/// SafeResourceRegistry
pub struct SafeResourceRegistry<S: Resource, P> {
    pub inner: Arc<RwLock<ResourceRegistry<S, P>>>,
}

impl<S: Resource, P> SafeResourceRegistry<S, P> {
    ///
    pub fn new(poll_registry: PollRegistry) -> Self {
        Self { inner: Arc::new(RwLock::new(ResourceRegistry::<S, P>::new(poll_registry))) }
    }

    /// Add a resource into the registry.
    pub fn register(&self, resource: S, properties: P, write_readiness: bool) -> ResourceId {
        let mut registry = self.inner.write();
        registry.register(resource, properties, write_readiness)
    }

    /// Remove a register from the registry.
    /// This function ensure that the register is removed from the registry,
    /// but not the destruction of the resource itself.
    /// Because the resource is shared, the destruction will be delayed until the last reference.
    pub fn deregister(&self, id: ResourceId) -> bool {
        let mut registry = self.inner.write();
        registry.deregister(id).is_some()
    }

    /// Returned a shared reference of the register.
    pub fn get(&self, id: ResourceId) -> Option<Arc<Register<S, P>>> {
        let registry = self.inner.read();
        registry.get(id)
    }
}

impl<S: Resource, P> Clone for SafeResourceRegistry<S, P> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

pub struct ResourceRegistry<S: Resource, P> {
    resources: HashMap<ResourceId, Arc<Register<S, P>>>,
    poll_registry: Arc<PollRegistry>,
}

impl<S: Resource, P> ResourceRegistry<S, P> {
    #[inline(always)]
    fn new(poll_registry: PollRegistry) -> Self {
        ResourceRegistry {
            //
            resources: HashMap::new(),
            poll_registry: Arc::new(poll_registry),
        }
    }

    //#[inline(always)]
    fn register(&mut self, mut resource: S, properties: P, write_readiness: bool) -> ResourceId {
        // The registry must be locked for the entire implementation to avoid the poll
        // to generate events over not yet registered resources.
        let id = self.poll_registry.add(resource.source(), write_readiness);
        let register = Register::new(resource, properties, self.poll_registry.clone());
        self.resources.insert(id, Arc::new(register));
        //log::info!("registry ++++ {:?} len={}", id, self.resources.len());
        id
    }

    //#[inline(always)]
    fn deregister(&mut self, id: ResourceId) -> Option<Arc<Register<S, P>>> {
        let regitser_opt = self.resources.remove(&id);
        //log::info!("registry ---- {:?} len={}", id, self.resources.len());
        regitser_opt
    }

    //#[inline(always)]
    fn get(&self, id: ResourceId) -> Option<Arc<Register<S, P>>> {
        self.resources.get(&id).cloned()
    }
}
