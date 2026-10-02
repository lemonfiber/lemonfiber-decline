//! The decline service: the one process that answers an invitation's decline address.
//!
//! It runs in the stack beside the household front door, restarted by the stack's
//! policy, and is the only thing that serves the decline address (ADR-0029). This
//! build answers its health and refuses every other request.

mod serving;

use std::net::SocketAddr;
use std::process::ExitCode;

/// Where the service listens inside its container. The stack publishes it at the
/// household binding tier; the port inside never changes.
const LISTEN: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8080);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => serving::serve(LISTEN).await,
        Some("health") => serving::healthy(LISTEN.port()).await,
        Some(other) => {
            eprintln!("decline: no command called {other}; run it bare to serve, or `health` to ask whether it is serving");
            ExitCode::from(2)
        }
    }
}
