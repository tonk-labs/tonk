pub mod manager;
pub mod discovery;
pub mod sync;
pub mod mdns;
pub mod connection;
pub mod network;
pub mod config;

#[cfg(test)]
mod tests;

pub use manager::P2PManager;