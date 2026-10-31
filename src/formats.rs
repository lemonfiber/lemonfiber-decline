//! The files the core and the decline service hand each other in its configuration
//! directory, and the shapes they share: the certificates the media server's internal TLS
//! port may present and a token held only as its hash.

pub mod decline;

mod pins;
mod shape;
mod token;
mod unreadable;

pub use pins::Pins;
pub use token::TokenHash;
pub use unreadable::Unreadable;
