//! The contract the service speaks to the core, on a port of its own.
//!
//! It answers `invite.decline` under the plugin's key, on a port published on loopback
//! alone, apart from the page the household reaches.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use lemonfiber_adapter_kit::Kit;
use lemonfiber_contract::adapter::About;
use lemonfiber_contract::capabilities::invite::decline;

use crate::holding::Holding;

/// What the service calls itself as the upstream it answers for.
pub(crate) const UPSTREAM: &str = "lemonfiber decline";

/// What the service says it is: the one contract it speaks, for itself.
pub(crate) fn about() -> About {
    About {
        speaks: vec![lemonfiber_contract::spoken(
            decline::CAPABILITY,
            decline::MAJOR,
        )],
        upstream: UPSTREAM.to_owned(),
        releases: Vec::new(),
    }
}

/// `invite.decline`, served under `key` from what is held in `config`.
pub(crate) fn kit(key: String, config: &Path) -> Kit {
    Kit::new(key, about()).serving(decline::served(Arc::new(Holding::in_directory(config))))
}

/// Answer the core on `at` under the plugin's key in `config`, for as long as the service
/// runs: it ends only where `at` cannot be served, with why.
///
/// Where the core has written no key, nothing answers there: a contract asked with no key
/// to check it against would answer anybody.
pub(crate) async fn serve(at: SocketAddr, config: &Path) -> std::io::Result<()> {
    let Some(key) = lemonfiber_adapter_kit::key_in(config)
        .ok()
        .filter(|key| !key.is_empty())
    else {
        eprintln!("decline: the core has written no key, so invite.decline is not answered");
        return std::future::pending().await;
    };
    let listener = tokio::net::TcpListener::bind(at).await?;
    lemonfiber_adapter_kit::serving(kit(key, config), listener, std::future::pending()).await
}

#[cfg(test)]
mod tests;
