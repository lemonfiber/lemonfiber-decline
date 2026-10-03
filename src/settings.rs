//! Where the service finds its files and the media server.

use std::path::PathBuf;

/// Where the service finds its files and the media server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    /// The configuration directory the stack mounts: the core's two files, read-only,
    /// and the refusals this service writes.
    pub(crate) config: PathBuf,
    /// The media server, on the one network the two share.
    pub(crate) jellyfin: String,
}

/// The configuration directory, unless `LEMONFIBER_DECLINE_CONFIG` names another.
const CONFIG: &str = "/config";

/// The media server, unless `LEMONFIBER_DECLINE_JELLYFIN` names another: the stack's
/// own service id and port for it.
const JELLYFIN: &str = "http://jellyfin:8096";

impl Settings {
    /// The settings `variable` gives, each falling back to the stack's default.
    pub(crate) fn from(variable: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            config: variable("LEMONFIBER_DECLINE_CONFIG")
                .map_or_else(|| PathBuf::from(CONFIG), PathBuf::from),
            jellyfin: variable("LEMONFIBER_DECLINE_JELLYFIN").map_or_else(
                || JELLYFIN.to_owned(),
                |base| base.trim_end_matches('/').to_owned(),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
