#![deny(unsafe_op_in_unsafe_fn)]

pub mod api_key;
pub mod container;
pub mod error;
pub mod http;
pub mod jwt;
pub mod refresh;

pub use zhs_json::{FromJson, ToJson};

pub(crate) fn recover_poison<T>(poisoned: std::sync::PoisonError<T>) -> T {
    eprintln!("zhs-http mutex poisoned; recovering");
    poisoned.into_inner()
}
