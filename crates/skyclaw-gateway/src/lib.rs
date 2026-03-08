//! SkyClaw Gateway crate — HTTP/WebSocket gateway that routes messages
//! between messaging channels and the agent runtime.

pub mod dashboard;
pub mod health;
pub mod identity;
pub mod router;
pub mod server;
pub mod session;

pub use identity::OAuthIdentityManager;
pub use server::SkyGate;
