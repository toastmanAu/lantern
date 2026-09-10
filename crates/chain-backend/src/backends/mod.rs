//! Concrete backends behind the [`crate::backend::ChainBackend`] trait.

pub mod embedded_light;
pub mod full_node;
pub mod remote_light;

pub use embedded_light::EmbeddedLight;
pub use full_node::FullNode;
pub use remote_light::RemoteLight;
