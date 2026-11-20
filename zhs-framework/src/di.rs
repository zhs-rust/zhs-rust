use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use zhs_entity::Database;
use zhs_http::{Controller, Router, ServiceResolver};

use crate::auth::AuthorizationOptions;
use crate::health::HealthCheckRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiError {
    MissingService(String),
    CircularDependency(String),
}

impl fmt::Display for DiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiError::MissingService(name) => {
                write!(formatter, "no service registered for `{name}`")
            }
            DiError::CircularDependency(name) => {
                write!(formatter, "circular dependency while creating `{name}`")
            }
        }
    }
}

impl std::error::Error for DiError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    Singleton,
    Scoped,
    Transient,
}

pub trait Injectable: Sized + Send + Sync + 'static {
    fn create(services: &dyn ServiceResolver) -> Result<Self, DiError>;
}

type Factory =
    Arc<dyn Fn(&dyn ServiceResolver) -> Result<Arc<dyn Any + Send + Sync>, DiError> + Send + Sync>;

struct Descriptor {
    lifetime: Lifetime,
    factory: Factory,
}

#[derive(Default)]
pub struct ServiceCollection {
    descriptors: HashMap<TypeId, Descriptor>,
    controllers: Vec<fn(&mut Router)>,
}

impl ServiceCollection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_singleton_instance<T>(&mut self, value: T) -> &mut Self
    where
        T: Any + Send + Sync,
    {
        let value: Arc<dyn Any + Send + Sync> = Arc::new(value);
        self.insert::<T>(
            Lifetime::Singleton,
            Arc::new(move |_| Ok(Arc::clone(&value))),
        )
    }

    pub fn add_singleton<T>(&mut self) -> &mut Self
    where
        T: Injectable,
    {
        self.insert::<T>(
            Lifetime::Singleton,
            Arc::new(|services| {
                let value = T::create(services)?;
                Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
            }),
        )
    }

    pub fn add_scoped<T>(&mut self) -> &mut Self
    where
        T: Injectable,
    {
        self.insert::<T>(
            Lifetime::Scoped,
            Arc::new(|services| {
                let value = T::create(services)?;
                Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
            }),
        )
    }

    pub fn add_transient<T>(&mut self) -> &mut Self
    where
        T: Injectable,
    {
        self.insert::<T>(
            Lifetime::Transient,
            Arc::new(|services| {
                let value = T::create(services)?;
                Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
            }),
        )
    }

    pub fn add_singleton_factory<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Any + Send + Sync,
        F: Fn(&dyn ServiceResolver) -> Result<T, DiError> + Send + Sync + 'static,
    {
        self.insert::<T>(
            Lifetime::Singleton,
            Arc::new(move |services| {
                let value = factory(services)?;
                Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
            }),
        )
    }

    pub fn add_scoped_factory<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Any + Send + Sync,
        F: Fn(&dyn ServiceResolver) -> Result<T, DiError> + Send + Sync + 'static,
    {
        self.insert::<T>(
            Lifetime::Scoped,
            Arc::new(move |services| {
                let value = factory(services)?;
                Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
            }),
        )
    }

    pub fn add_controllers<T>(&mut self) -> &mut Self
    where
        T: Controller + Injectable,
    {
        self.controllers.push(T::register);
        self.add_scoped::<T>()
    }

    pub fn add_database(&mut self, database: Database) -> &mut Self {
        self.add_singleton_instance(database)
    }

    pub fn add_authorization<F>(&mut self, configure: F) -> &mut Self
    where
        F: FnOnce(&mut AuthorizationOptions),
    {
        let mut options = AuthorizationOptions::default();
        configure(&mut options);
        self.add_singleton_instance(options)
    }

    pub fn add_health_checks<F>(&mut self, configure: F) -> &mut Self
    where
        F: FnOnce(&mut HealthCheckRegistry),
    {
        let mut registry = HealthCheckRegistry::new();
        configure(&mut registry);
        self.add_singleton_instance(registry)
    }

    pub fn build(self) -> Result<ServiceProvider, DiError> {
        Ok(ServiceProvider {
            inner: Arc::new(ProviderInner {
                descriptors: self.descriptors,
                singletons: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn take_controllers(&mut self) -> Vec<fn(&mut Router)> {
        std::mem::take(&mut self.controllers)
    }

    fn insert<T>(&mut self, lifetime: Lifetime, factory: Factory) -> &mut Self
    where
        T: Any + Send + Sync,
    {
        self.descriptors
            .insert(TypeId::of::<T>(), Descriptor { lifetime, factory });
        self
    }
}

#[derive(Clone)]
pub struct ServiceProvider {
    inner: Arc<ProviderInner>,
}

struct ProviderInner {
    descriptors: HashMap<TypeId, Descriptor>,
    singletons: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

thread_local! {
    static CREATING: RefCell<HashSet<TypeId>> = RefCell::new(HashSet::new());
}

impl ServiceProvider {
    pub fn get<T>(&self) -> Option<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        self.resolve_any(TypeId::of::<T>())
            .and_then(|value| value.downcast::<T>().ok())
    }

    pub fn get_required<T>(&self) -> Result<Arc<T>, DiError>
    where
        T: Any + Send + Sync,
    {
        let type_name = std::any::type_name::<T>();
        let value = self
            .resolve_any(TypeId::of::<T>())
            .ok_or_else(|| DiError::MissingService(type_name.to_owned()))?;

        value
            .downcast::<T>()
            .map_err(|_| DiError::MissingService(type_name.to_owned()))
    }

    fn resolve_any(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.inner.resolve(type_id, self, None)
    }
}

impl ServiceResolver for ServiceProvider {
    fn resolve(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.resolve_any(type_id)
    }

    fn create_scope(&self) -> Arc<dyn ServiceResolver> {
        Arc::new(RequestScope {
            root: Arc::clone(&self.inner),
            scoped: Mutex::new(HashMap::new()),
        })
    }
}

struct RequestScope {
    root: Arc<ProviderInner>,
    scoped: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl ServiceResolver for RequestScope {
    fn resolve(&self, type_id: TypeId) -> Option<Arc<dyn Any + Send + Sync>> {
        self.root.resolve(type_id, self, Some(&self.scoped))
    }

    fn create_scope(&self) -> Arc<dyn ServiceResolver> {
        Arc::new(RequestScope {
            root: Arc::clone(&self.root),
            scoped: Mutex::new(HashMap::new()),
        })
    }
}

impl ProviderInner {
    fn resolve(
        &self,
        type_id: TypeId,
        resolver: &dyn ServiceResolver,
        scoped: Option<&Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let descriptor = self.descriptors.get(&type_id)?;

        match descriptor.lifetime {
            Lifetime::Singleton => {
                if let Some(instance) = self
                    .singletons
                    .lock()
                    .unwrap_or_else(|poisoned| {
                        eprintln!("singleton cache mutex poisoned; recovering");
                        poisoned.into_inner()
                    })
                    .get(&type_id)
                    .cloned()
                {
                    return Some(instance);
                }

                let created = self.create(type_id, descriptor, resolver)?;

                // Concurrent first resolutions may both construct the
                // service; the cache arbitrates so every caller observes
                // the same instance instead of the last one overwriting
                // the others.
                let mut singletons = self.singletons.lock().unwrap_or_else(|poisoned| {
                    eprintln!("singleton cache mutex poisoned; recovering");
                    poisoned.into_inner()
                });
                let instance = singletons.entry(type_id).or_insert(created);
                Some(Arc::clone(instance))
            }
            Lifetime::Scoped => {
                let scoped = scoped?;

                if let Some(instance) = scoped
                    .lock()
                    .unwrap_or_else(|poisoned| {
                        eprintln!("scoped cache mutex poisoned; recovering");
                        poisoned.into_inner()
                    })
                    .get(&type_id)
                    .cloned()
                {
                    return Some(instance);
                }

                let created = self.create(type_id, descriptor, resolver)?;
                let mut scoped = scoped.lock().unwrap_or_else(|poisoned| {
                    eprintln!("scoped cache mutex poisoned; recovering");
                    poisoned.into_inner()
                });
                let instance = scoped.entry(type_id).or_insert(created);
                Some(Arc::clone(instance))
            }
            Lifetime::Transient => self.create(type_id, descriptor, resolver),
        }
    }

    fn create(
        &self,
        type_id: TypeId,
        descriptor: &Descriptor,
        resolver: &dyn ServiceResolver,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let inserted = CREATING.with(|creating| creating.borrow_mut().insert(type_id));
        if !inserted {
            // Circular dependency within the current resolution stack.
            return None;
        }

        struct Guard(TypeId);
        impl Drop for Guard {
            fn drop(&mut self) {
                CREATING.with(|creating| {
                    creating.borrow_mut().remove(&self.0);
                });
            }
        }
        let _guard = Guard(type_id);

        match (descriptor.factory)(resolver) {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("service creation failed: {error}");
                None
            }
        }
    }
}

impl fmt::Debug for ServiceProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceProvider")
            .field("services", &self.inner.descriptors.len())
            .finish()
    }
}
