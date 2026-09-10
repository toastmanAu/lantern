//! Concrete backends behind the [`crate::backend::ChainBackend`] trait.

pub mod remote_light;

pub use remote_light::RemoteLight;
