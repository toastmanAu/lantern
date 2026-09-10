//! Concrete backends behind the [`crate::backend::ChainBackend`] trait.

pub mod full_node;
pub mod remote_light;

pub use full_node::FullNode;
pub use remote_light::RemoteLight;
