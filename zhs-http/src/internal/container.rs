use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::internal::error::{ContainerError, Result};

type BoxedAny = Box<dyn Any + Send + Sync>;
type ErasedFactory = dyn for<'a> Fn(&ServiceProvider<'a>) -> Result<BoxedService> + Send + Sync;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lifetime {
    Singleton,

    Scoped,

    Transient,
}

impl Lifetime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Lifetime::Singleton => "singleton",
            Lifetime::Scoped => "scoped",
            Lifetime::Transient => "transient",
        }
    }
}

impl fmt::Display for Lifetime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

struct BoxedService {
    value: BoxedAny,
    clone_box: fn(&BoxedAny) -> BoxedAny,
}

impl BoxedService {
    fn new<T>(value: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        fn clone_box<T>(value: &BoxedAny) -> BoxedAny
        where
            T: Clone + Send + Sync + 'static,
        {
            let value = value
                .downcast_ref::<T>()
                .expect("boxed service type should match its clone function");
            Box::new(value.clone())
        }

        Self {
            value: Box::new(value),
            clone_box: clone_box::<T>,
        }
    }

    fn into_arc<T>(self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.value
            .downcast::<Arc<T>>()
            .map(|value| *value)
            .map_err(|_| ContainerError::type_mismatch::<T>().into())
    }

    fn arc<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.value
            .downcast_ref::<Arc<T>>()
            .map(Arc::clone)
            .ok_or_else(|| ContainerError::type_mismatch::<T>().into())
    }
}

impl Clone for BoxedService {
    fn clone(&self) -> Self {
        Self {
            value: (self.clone_box)(&self.value),
            clone_box: self.clone_box,
        }
    }
}

impl fmt::Debug for BoxedService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoxedService(..)")
    }
}

struct Service {
    type_id: TypeId,
    name: String,
    type_name: &'static str,
    lifetime: Lifetime,
    kind: ServiceKind,
}

enum ServiceKind {
    Instance(BoxedService),
    Factory(Arc<ErasedFactory>),
}

impl Service {
    fn describe(&self) -> String {
        format!("{} ({}, {})", self.name, self.type_name, self.lifetime)
    }

    fn factory(&self) -> &ErasedFactory {
        match &self.kind {
            ServiceKind::Factory(factory) => factory.as_ref(),
            ServiceKind::Instance(_) => {
                unreachable!("only factory registrations are resolved lazily")
            }
        }
    }
}

impl fmt::Debug for Service {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Service")
            .field("name", &self.name)
            .field("type_name", &self.type_name)
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

pub struct ServiceProvider<'a> {
    inner: ProviderInner<'a>,
}

enum ProviderInner<'a> {
    Build(&'a BuildContext<'a>),
    Runtime {
        container: &'a Container,
        scope: Option<&'a ServiceScope>,
    },
}

impl ServiceProvider<'_> {
    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        match &self.inner {
            ProviderInner::Build(context) => context.resolve(self),
            ProviderInner::Runtime { container, scope } => resolve_runtime(container, *scope, self),
        }
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.resolve().ok()
    }

    pub fn contains<T>(&self) -> bool
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.lifetime::<T>().is_some()
    }

    pub fn lifetime<T>(&self) -> Option<Lifetime>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        match &self.inner {
            ProviderInner::Build(context) => context
                .services
                .iter()
                .find(|service| service.type_id == type_id)
                .map(|service| service.lifetime),
            ProviderInner::Runtime { container, .. } => container.lifetime::<T>(),
        }
    }
}

struct BuildContext<'a> {
    services: &'a [Service],
    cache: RefCell<HashMap<TypeId, BoxedService>>,
    building: RefCell<HashSet<TypeId>>,
}

impl BuildContext<'_> {
    fn resolve<T>(&self, _provider: &ServiceProvider<'_>) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let service = self
            .services
            .iter()
            .find(|service| service.type_id == type_id)
            .ok_or_else(ContainerError::missing::<T>)?;

        match &service.kind {
            ServiceKind::Instance(value) => value.arc::<T>(),
            ServiceKind::Factory(_) => {
                if service.lifetime != Lifetime::Singleton {
                    return Err(ContainerError::scope_required(service.type_name).into());
                }
                self.ensure_singleton(type_id, service.type_name)?;
                let cached = self
                    .cache
                    .borrow()
                    .get(&type_id)
                    .expect("an ensured singleton should be cached")
                    .clone();
                cached.into_arc::<T>()
            }
        }
    }

    fn ensure_singleton(&self, type_id: TypeId, type_name: &'static str) -> Result<()> {
        if self.cache.borrow().contains_key(&type_id) {
            return Ok(());
        }

        let service = self
            .services
            .iter()
            .find(|service| service.type_id == type_id)
            .ok_or_else(|| {
                ContainerError::new(format!(
                    "service `{type_name}` is not registered in the container"
                ))
            })?;

        if service.lifetime != Lifetime::Singleton {
            return Err(ContainerError::scope_required(service.type_name).into());
        }

        let factory = match &service.kind {
            ServiceKind::Factory(factory) => Arc::clone(factory),
            ServiceKind::Instance(_) => return Ok(()),
        };

        if !self.building.borrow_mut().insert(type_id) {
            return Err(ContainerError::circular(type_name).into());
        }

        let provider = ServiceProvider {
            inner: ProviderInner::Build(self),
        };
        let result = factory(&provider);
        self.building.borrow_mut().remove(&type_id);

        let value = result?;
        self.cache.borrow_mut().insert(type_id, value);
        Ok(())
    }

    fn finish(self) -> HashMap<TypeId, BoxedService> {
        self.cache.into_inner()
    }
}

fn resolve_runtime<T>(
    container: &Container,
    scope: Option<&ServiceScope>,
    provider: &ServiceProvider<'_>,
) -> Result<Arc<T>>
where
    T: ?Sized + Send + Sync + 'static,
{
    let type_id = TypeId::of::<T>();

    if let Some(scope) = scope
        && let Some(cached) = scope.cached(type_id)
    {
        return cached.into_arc::<T>();
    }

    if let Some(value) = container.instances.get_boxed(type_id) {
        return value.into_arc::<T>();
    }

    let Some(service) = container.factories.get(&type_id) else {
        return Err(ContainerError::missing::<T>().into());
    };

    let scope = scope.ok_or_else(|| ContainerError::scope_required(service.type_name))?;

    if service.lifetime == Lifetime::Scoped
        && let Some(cached) = scope.cached(type_id)
    {
        return cached.into_arc::<T>();
    }

    let guard = scope.enter(type_id, service.type_name)?;
    let created = service.factory()(provider);
    drop(guard);

    let created = created?;

    if service.lifetime == Lifetime::Scoped {
        return scope.store(type_id, created).into_arc::<T>();
    }

    created.into_arc::<T>()
}

#[derive(Default)]
pub struct TypeMap {
    entries: HashMap<TypeId, Entry>,
}

struct Entry {
    name: String,
    type_name: &'static str,
    value: BoxedService,
}

impl Entry {
    fn describe(&self) -> String {
        format!("{} ({})", self.name, self.type_name)
    }
}

impl TypeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn register<T>(&mut self, name: impl Into<String>, value: Arc<T>)
    where
        T: ?Sized + Send + Sync + 'static,
    {
        let name = name.into();
        let type_id = TypeId::of::<T>();

        if let Some(previous) = self.entries.get(&type_id) {
            panic!(
                "service `{}` ({}) is already registered",
                previous.name, previous.type_name
            );
        }

        self.entries.insert(
            type_id,
            Entry {
                name,
                type_name: std::any::type_name::<T>(),
                value: BoxedService::new(value),
            },
        );
    }

    fn upsert<T>(&mut self, value: Arc<T>)
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.entries.insert(
            TypeId::of::<T>(),
            Entry {
                name: std::any::type_name::<T>().to_owned(),
                type_name: std::any::type_name::<T>(),
                value: BoxedService::new(value),
            },
        );
    }

    fn insert_boxed(
        &mut self,
        type_id: TypeId,
        name: impl Into<String>,
        type_name: &'static str,
        value: BoxedService,
    ) {
        self.entries.insert(
            type_id,
            Entry {
                name: name.into(),
                type_name,
                value,
            },
        );
    }

    pub fn get<T>(&self) -> Option<&T>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.entries
            .get(&TypeId::of::<T>())
            .and_then(|entry| entry.value.value.downcast_ref::<Arc<T>>())
            .map(|service| &**service)
    }

    fn get_arc<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.entries
            .get(&TypeId::of::<T>())
            .and_then(|entry| entry.value.arc::<T>().ok())
    }

    fn get_boxed(&self, type_id: TypeId) -> Option<BoxedService> {
        self.entries.get(&type_id).map(|entry| entry.value.clone())
    }

    pub fn require<T>(&self) -> Result<&T>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.get::<T>()
            .ok_or_else(|| ContainerError::missing::<T>().into())
    }

    pub fn describe(&self) -> Vec<String> {
        let mut result: Vec<String> = self.entries.values().map(Entry::describe).collect();
        result.sort();
        result
    }
}

impl fmt::Debug for TypeMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.entries
                    .values()
                    .map(|entry| (&entry.name, &entry.type_name)),
            )
            .finish()
    }
}

#[derive(Default)]
pub struct Builder {
    services: Vec<Service>,
}

impl Builder {
    pub fn register<T>(mut self, name: impl Into<String>, value: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.push(Service {
            type_id: TypeId::of::<T>(),
            name: name.into(),
            type_name: std::any::type_name::<T>(),
            lifetime: Lifetime::Singleton,
            kind: ServiceKind::Instance(BoxedService::new(value)),
        });
        self
    }

    pub fn factory<T, F>(mut self, name: impl Into<String>, lifetime: Lifetime, factory: F) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: for<'a> Fn(&ServiceProvider<'a>) -> Result<Arc<T>> + Send + Sync + 'static,
    {
        let erased: Arc<ErasedFactory> = Arc::new(move |provider: &ServiceProvider<'_>| {
            let value = factory(provider)?;
            Ok(BoxedService::new(value))
        });

        self.push(Service {
            type_id: TypeId::of::<T>(),
            name: name.into(),
            type_name: std::any::type_name::<T>(),
            lifetime,
            kind: ServiceKind::Factory(erased),
        });
        self
    }

    pub fn singleton<T, F>(self, name: impl Into<String>, factory: F) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: for<'a> Fn(&ServiceProvider<'a>) -> Result<Arc<T>> + Send + Sync + 'static,
    {
        self.factory(name, Lifetime::Singleton, factory)
    }

    pub fn scoped<T, F>(self, name: impl Into<String>, factory: F) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: for<'a> Fn(&ServiceProvider<'a>) -> Result<Arc<T>> + Send + Sync + 'static,
    {
        self.factory(name, Lifetime::Scoped, factory)
    }

    pub fn transient<T, F>(self, name: impl Into<String>, factory: F) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: for<'a> Fn(&ServiceProvider<'a>) -> Result<Arc<T>> + Send + Sync + 'static,
    {
        self.factory(name, Lifetime::Transient, factory)
    }

    fn push(&mut self, service: Service) {
        if let Some(previous) = self
            .services
            .iter()
            .find(|previous| previous.type_id == service.type_id)
        {
            panic!(
                "service `{}` ({}) is already registered",
                previous.name, previous.type_name
            );
        }

        self.services.push(service);
    }

    pub fn build(self) -> Result<Arc<Container>> {
        let services = self.services;
        let context = BuildContext {
            services: &services,
            cache: RefCell::new(HashMap::new()),
            building: RefCell::new(HashSet::new()),
        };

        for service in &services {
            if service.lifetime == Lifetime::Singleton
                && matches!(&service.kind, ServiceKind::Factory(_))
            {
                context.ensure_singleton(service.type_id, service.type_name)?;
            }
        }
        let mut created = context.finish();

        let mut instances = TypeMap::new();
        let mut factories = HashMap::new();

        for service in services {
            match service.lifetime {
                Lifetime::Singleton => {
                    let value = match service.kind {
                        ServiceKind::Instance(value) => value,
                        ServiceKind::Factory(_) => {
                            created.remove(&service.type_id).ok_or_else(|| {
                                ContainerError::new(format!(
                                    "singleton `{}` was not initialized",
                                    service.type_name
                                ))
                            })?
                        }
                    };
                    instances.insert_boxed(service.type_id, service.name, service.type_name, value);
                }
                lifetime => {
                    factories.insert(
                        service.type_id,
                        Service {
                            lifetime,
                            ..service
                        },
                    );
                }
            }
        }

        Ok(Arc::new(Container {
            instances,
            factories,
        }))
    }
}

pub struct Container {
    instances: TypeMap,
    factories: HashMap<TypeId, Service>,
}

impl Container {
    pub fn builder() -> Builder {
        Builder::default()
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.resolve::<T>().ok()
    }

    pub fn require<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.resolve::<T>()
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provider().resolve::<T>()
    }

    pub fn contains<T>(&self) -> bool
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.lifetime::<T>().is_some()
    }

    pub fn lifetime<T>(&self) -> Option<Lifetime>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        if self.instances.entries.contains_key(&TypeId::of::<T>()) {
            Some(Lifetime::Singleton)
        } else {
            self.factories
                .get(&TypeId::of::<T>())
                .map(|service| service.lifetime)
        }
    }

    pub fn create_scope(self: &Arc<Self>) -> ServiceScope {
        ServiceScope::new(Arc::clone(self))
    }

    pub fn provider(&self) -> ServiceProvider<'_> {
        ServiceProvider {
            inner: ProviderInner::Runtime {
                container: self,
                scope: None,
            },
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len() + self.factories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn describe(&self) -> Vec<String> {
        let mut result: Vec<String> = self
            .instances
            .entries
            .values()
            .map(|entry| format!("{} ({}, singleton)", entry.name, entry.type_name))
            .collect();
        result.extend(self.factories.values().map(Service::describe));
        result.sort();
        result
    }
}

impl fmt::Debug for Container {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Container")
            .field("instances", &self.instances)
            .field("factories", &self.factories.len())
            .finish()
    }
}

pub struct ServiceScope {
    container: Arc<Container>,
    scoped: Mutex<HashMap<TypeId, BoxedService>>,
    building: Mutex<HashSet<TypeId>>,
}

impl ServiceScope {
    fn new(container: Arc<Container>) -> Self {
        Self {
            container,
            scoped: Mutex::new(HashMap::new()),
            building: Mutex::new(HashSet::new()),
        }
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    pub fn provider(&self) -> ServiceProvider<'_> {
        ServiceProvider {
            inner: ProviderInner::Runtime {
                container: self.container.as_ref(),
                scope: Some(self),
            },
        }
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provider().resolve::<T>()
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.resolve::<T>().ok()
    }

    pub fn require<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.resolve::<T>()
    }

    fn cached(&self, type_id: TypeId) -> Option<BoxedService> {
        self.scoped
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
            .get(&type_id)
            .cloned()
    }

    fn store(&self, type_id: TypeId, value: BoxedService) -> BoxedService {
        self.scoped
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
            .entry(type_id)
            .or_insert(value)
            .clone()
    }

    fn enter(&self, type_id: TypeId, type_name: &'static str) -> Result<ScopeGuard<'_>> {
        let mut building = self
            .building
            .lock()
            .unwrap_or_else(crate::internal::recover_poison);

        if !building.insert(type_id) {
            return Err(ContainerError::circular(type_name).into());
        }

        Ok(ScopeGuard {
            scope: self,
            type_id,
        })
    }
}

impl fmt::Debug for ServiceScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceScope")
            .field("container", &self.container.len())
            .field(
                "scoped",
                &self
                    .scoped
                    .lock()
                    .map(|scoped| scoped.len())
                    .unwrap_or_default(),
            )
            .finish()
    }
}

struct ScopeGuard<'a> {
    scope: &'a ServiceScope,
    type_id: TypeId,
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        self.scope
            .building
            .lock()
            .unwrap_or_else(crate::internal::recover_poison)
            .remove(&self.type_id);
    }
}

#[derive(Debug, Default)]
pub struct Scope {
    services: TypeMap,
}

impl Scope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T>(&mut self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.services.upsert(Arc::new(value));
    }

    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.services.get_arc::<T>()
    }

    pub fn require<T>(&self) -> Result<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.get::<T>()
            .ok_or_else(|| ContainerError::missing::<T>().into())
    }

    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    pub fn len(&self) -> usize {
        self.services.len()
    }
}
