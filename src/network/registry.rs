use hashbrown::HashMap;
use std::cell::UnsafeCell;
use std::rc::Rc;

use super::adapter::Resource;
use super::poll::PollRegistry;
use super::resource_id::ResourceId;

/// Register of resource
pub struct Register<S: Resource, P> {
    pub resource: S,
    pub properties: P,

    poll_registry: Rc<PollRegistry>,
}

impl<S: Resource, P> Register<S, P> {
    #[inline(always)]
    fn new(resource: S, properties: P, poll_registry: Rc<PollRegistry>) -> Self {
        Self { resource, properties, poll_registry }
    }
}

impl<S: Resource, P> Drop for Register<S, P> {
    #[inline(always)]
    fn drop(&mut self) {
        self.poll_registry.remove(self.resource.source());
    }
}

/// LocklessResourceRegistry
pub struct LocklessResourceRegistry<S: Resource, P> {
    pub inner: Rc<UnsafeCell<ResourceRegistry<S, P>>>,
}

impl<S: Resource, P> LocklessResourceRegistry<S, P> {
    ///
    pub fn new(poll_registry: PollRegistry) -> Self {
        Self { inner: Rc::new(UnsafeCell::new(ResourceRegistry::<S, P>::new(poll_registry))) }
    }

    /// Add a resource into the registry.
    #[inline(always)]
    pub fn register(&self, resource: S, properties: P, write_readiness: bool) -> ResourceId {
        let registry = unsafe { &mut *self.inner.get() };
        registry.register(resource, properties, write_readiness)
    }

    /// Remove a register from the registry.
    /// This function ensure that the register is removed from the registry,
    /// but not the destruction of the resource itself.
    /// Because the resource is shared, the destruction will be delayed until the last reference.
    #[inline(always)]
    pub fn deregister(&self, id: ResourceId) -> bool {
        let registry = unsafe { &mut *self.inner.get() };
        registry.deregister(id).is_some()
    }

    /// Returned a shared reference of the register.
    #[inline(always)]
    pub fn get(&self, id: ResourceId) -> Option<Rc<Register<S, P>>> {
        let registry = unsafe { &mut *self.inner.get() };
        registry.get(id)
    }
}

impl<S: Resource, P> Clone for LocklessResourceRegistry<S, P> {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

pub struct ResourceRegistry<S: Resource, P> {
    resources: HashMap<ResourceId, Rc<Register<S, P>>>,
    poll_registry: Rc<PollRegistry>,
}

impl<S: Resource, P> ResourceRegistry<S, P> {
    #[inline(always)]
    fn new(poll_registry: PollRegistry) -> Self {
        ResourceRegistry {
            //
            resources: HashMap::new(),
            poll_registry: Rc::new(poll_registry),
        }
    }

    #[inline(always)]
    fn register(&mut self, mut resource: S, properties: P, write_readiness: bool) -> ResourceId {
        // to generate events over not yet registered resources.
        let id = self.poll_registry.add(resource.source(), write_readiness);
        let register = Register::new(resource, properties, self.poll_registry.clone());
        self.resources.insert(id, Rc::new(register));
        //log::info!("registry ++++ {:?} len={}", id, self.resources.len());
        id
    }

    #[inline(always)]
    fn deregister(&mut self, id: ResourceId) -> Option<Rc<Register<S, P>>> {
        let regitser_opt = self.resources.remove(&id);
        //log::info!("registry ---- {:?} len={}", id, self.resources.len());
        regitser_opt
    }

    #[inline(always)]
    fn get(&self, id: ResourceId) -> Option<Rc<Register<S, P>>> {
        self.resources.get(&id).cloned()
    }
}
