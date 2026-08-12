pub mod client;
pub mod compatibility;
pub mod observation;
pub mod protocol;
pub mod session;
pub mod tools;
pub mod transport;
pub mod types;

pub use client::ArenaGateway;
pub use transport::{HttpMcpTransport, McpError, McpTransport};
