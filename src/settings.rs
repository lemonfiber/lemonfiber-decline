//! Where the service finds its files and the media server.

use std::path::PathBuf;

use lemonfiber_pinned::Https;

/// Where the service finds its files and the media server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// The configuration directory the stack mounts: the core's two files, read-only,
    /// and the refusals this service writes.
    pub(crate) config: PathBuf,
    /// The media server's internal TLS port, on the one network the two share.
    pub(crate) jellyfin: Https,
}

/// The configuration directory, unless `LEMONFIBER_DECLINE_CONFIG` names another.
const CONFIG: &str = "/config";

/// The media server, unless `LEMONFIBER_DECLINE_JELLYFIN` names another: the stack's
/// own service id and the internal TLS port it serves the core's certificate on.
const JELLYFIN: &str = "https://jellyfin:8920";

impl Settings {
    /// The settings `variable` gives, each falling back to the stack's default, or why
    /// they cannot be: the media server is reached at an HTTPS address alone.
    pub(crate) fn from(variable: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let named = variable("LEMONFIBER_DECLINE_JELLYFIN").unwrap_or_else(|| JELLYFIN.to_owned());
        let Some(jellyfin) = Https::parse(&named) else {
            return Err(format!(
                "LEMONFIBER_DECLINE_JELLYFIN is {named:?}, and the media server is reached at an HTTPS address alone"
            ));
        };
        Ok(Self {
            config: variable("LEMONFIBER_DECLINE_CONFIG")
                .map_or_else(|| PathBuf::from(CONFIG), PathBuf::from),
            jellyfin,
        })
    }
}

#[cfg(test)]
mod tests;
