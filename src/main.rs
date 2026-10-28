//! The decline service: the one process that answers an invitation's decline address.
//!
//! It runs in the stack beside the household front door, restarted by the stack's
//! policy, and is the only thing that serves the decline address. It shows the
//! invitation a token names, and on a refusal disables the account made for it and
//! records the refusal for the core. Once a minute it takes back every invitation whose
//! window has closed, and records what it did. It speaks `invite.decline` to the core on a
//! port of its own.

mod declining;
mod holding;
mod jellyfin;
mod lapsing;
mod limit;
mod page;
mod serving;
mod settings;
mod speaking;

use std::net::SocketAddr;
use std::process::ExitCode;

/// Where the service answers the household inside its container. The stack publishes it
/// at the household binding tier; the port inside never changes.
const LISTEN: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8080);

/// Where the service speaks `invite.decline` inside its container, which the stack
/// publishes on loopback alone.
const CONTRACT: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 8081);

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {
            let settings = match settings::Settings::from(&|name| std::env::var(name).ok()) {
                Ok(settings) => settings,
                Err(why) => {
                    eprintln!("decline: {why}");
                    return ExitCode::from(2);
                }
            };
            let config = settings.config.clone();
            let service = serving::Service::new(settings, limit::Limit::standard());
            let stopped = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            tokio::select! {
                code = serving::serve(LISTEN, service.clone(), stopped) => code,
                () = serving::lapsing(service) => ExitCode::FAILURE,
                stopped = speaking::serve(CONTRACT, &config) => {
                    eprintln!("decline: invite.decline is not answered on {CONTRACT}: {stopped:?}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("health") => serving::healthy(LISTEN.port()).await,
        Some(other) => {
            eprintln!(
                "decline: no command called {other}; run it bare to serve, or `health` to ask whether it is serving"
            );
            ExitCode::from(2)
        }
    }
}
