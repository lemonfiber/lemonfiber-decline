//! The calls the service makes to the media server, and nothing else.
//!
//! It reads an account, reads whether that account's password moved since the
//! invitation was issued, writes the account's policy back disabled, and removes an
//! account. Each call is made server-side for one account the core's table names;
//! nothing a browser sends is passed through.

use serde_json::Value;

use lemonfiber_sidecar::decline::Key;

/// An account as the service needs it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Account {
    /// The identifier the server answered with, where it answered with one.
    pub(crate) id: Option<String>,
    /// Whether the server says a password is set on it, where it says.
    pub(crate) has_password: Option<bool>,
    /// Whether anybody has ever signed in to it or used it.
    pub(crate) seen: bool,
    /// Whether it administers the server, which is never declined.
    pub(crate) administrator: bool,
    /// Whether it is already disabled.
    pub(crate) disabled: bool,
    /// Its policy, whole, to write back with one field changed.
    pub(crate) policy: Value,
}

/// The media server did not answer, or answered something that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Silent;

/// What the service asks of the media server.
pub(crate) trait Server {
    /// The account `id`, where the server holds one.
    async fn account(&self, id: &str) -> Result<Option<Account>, Silent>;

    /// Whether the account `id`'s password moved after `since`, in seconds since the
    /// Unix epoch: a claim, by the server's own record of it.
    async fn claimed_since(&self, id: &str, since: u64) -> Result<bool, Silent>;

    /// Write `policy` back for the account `id`, with `IsDisabled` set.
    async fn disable(&self, id: &str, policy: Value) -> Result<(), Silent>;

    /// Remove the account `id`.
    async fn remove(&self, id: &str) -> Result<(), Silent>;
}

/// The media server over HTTP, with the key minted for this service.
pub(crate) struct Jellyfin {
    /// Where the server answers on the network the two share.
    pub(crate) base: String,
    /// The key minted for this service alone.
    pub(crate) key: Key,
    /// The client every call is made with.
    pub(crate) client: reqwest::Client,
}

/// What the server calls a password being set on an account, or taken off it.
const PASSWORD_MOVED: &str = "UserPasswordChanged";

impl Jellyfin {
    /// The header every call carries: the one scheme every supported line accepts.
    fn authorisation(&self) -> String {
        format!(
            "MediaBrowser Client=\"lemonfiber-decline\", Device=\"lemonfiber-decline\", \
             DeviceId=\"lemonfiber-decline\", Version=\"{}\", Token=\"{}\"",
            env!("CARGO_PKG_VERSION"),
            self.key.reveal()
        )
    }

    async fn get(&self, path: &str) -> Result<Option<Value>, Silent> {
        let answer = self
            .client
            .get(format!("{}{path}", self.base))
            .header("Authorization", self.authorisation())
            .send()
            .await
            .map_err(|_| Silent)?;
        if answer.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !answer.status().is_success() {
            return Err(Silent);
        }
        answer.json().await.map(Some).map_err(|_| Silent)
    }
}

impl Server for Jellyfin {
    async fn account(&self, id: &str) -> Result<Option<Account>, Silent> {
        let Some(user) = self.get(&format!("/Users/{id}")).await? else {
            return Ok(None);
        };
        account(&user).map(Some).ok_or(Silent)
    }

    async fn claimed_since(&self, id: &str, since: u64) -> Result<bool, Silent> {
        let path = format!("/System/ActivityLog/Entries?minDate={}", iso(since));
        let entries = self.get(&path).await?.ok_or(Silent)?;
        Ok(claimed(&entries, id, since))
    }

    async fn disable(&self, id: &str, policy: Value) -> Result<(), Silent> {
        let answer = self
            .client
            .post(format!("{}/Users/{id}/Policy", self.base))
            .header("Authorization", self.authorisation())
            .json(&disabled(policy))
            .send()
            .await
            .map_err(|_| Silent)?;
        succeeded(&answer)
    }

    async fn remove(&self, id: &str) -> Result<(), Silent> {
        let answer = self
            .client
            .delete(format!("{}/Users/{id}", self.base))
            .header("Authorization", self.authorisation())
            .send()
            .await
            .map_err(|_| Silent)?;
        succeeded(&answer)
    }
}

/// Whether the server took a write.
fn succeeded(answer: &reqwest::Response) -> Result<(), Silent> {
    if answer.status().is_success() {
        Ok(())
    } else {
        Err(Silent)
    }
}

/// The account a `/Users/{id}` answer describes.
fn account(user: &Value) -> Option<Account> {
    let policy = user.get("Policy")?.clone();
    let dated = |field: &str| {
        user.get(field)
            .and_then(Value::as_str)
            .is_some_and(|date| !date.is_empty())
    };
    Some(Account {
        id: user.get("Id").and_then(Value::as_str).map(str::to_owned),
        has_password: user.get("HasPassword").and_then(Value::as_bool),
        seen: dated("LastLoginDate") || dated("LastActivityDate"),
        administrator: policy.get("IsAdministrator")?.as_bool()?,
        disabled: policy
            .get("IsDisabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        policy,
    })
}

/// Whether `entries` record the account `id`'s password moving after `since`.
fn claimed(entries: &Value, id: &str, since: u64) -> bool {
    let Some(items) = entries.get("Items").and_then(Value::as_array) else {
        return false;
    };
    items.iter().any(|entry| {
        entry.get("Type").and_then(Value::as_str) == Some(PASSWORD_MOVED)
            && entry.get("UserId").and_then(Value::as_str).map(normalised) == Some(normalised(id))
            && entry
                .get("Date")
                .and_then(Value::as_str)
                .and_then(seconds)
                .is_some_and(|at| at > since)
    })
}

/// An account identifier without the hyphens one form of it carries.
pub(crate) fn normalised(id: &str) -> String {
    id.replace('-', "").to_ascii_lowercase()
}

/// `policy` with `IsDisabled` set and every other field as it was.
fn disabled(mut policy: Value) -> Value {
    if let Some(fields) = policy.as_object_mut() {
        fields.insert("IsDisabled".to_owned(), Value::Bool(true));
    }
    policy
}

/// `seconds` since the Unix epoch as the server takes a date, in RFC 3339.
fn iso(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|at| jiff::Timestamp::from_second(at).ok())
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_string()
}

/// The seconds since the Unix epoch a server date names, to the second.
fn seconds(date: &str) -> Option<u64> {
    let at: jiff::Timestamp = date.parse().ok()?;
    u64::try_from(at.as_second()).ok()
}

#[cfg(test)]
mod tests;
