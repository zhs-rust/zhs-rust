use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerError {
    message: String,
}

impl ContainerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn missing<T: ?Sized + 'static>() -> Self {
        Self::new(format!(
            "service `{}` is not registered in the container",
            std::any::type_name::<T>()
        ))
    }

    pub fn scope_required(type_name: &'static str) -> Self {
        Self::new(format!(
            "service `{type_name}` has a scoped or transient lifetime and requires a scope"
        ))
    }

    pub fn circular(type_name: &'static str) -> Self {
        Self::new(format!(
            "circular dependency detected while creating `{type_name}`"
        ))
    }

    pub fn type_mismatch<T: ?Sized + 'static>() -> Self {
        Self::new(format!(
            "service `{}` was registered with an unexpected type",
            std::any::type_name::<T>()
        ))
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ContainerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContainerError {}
