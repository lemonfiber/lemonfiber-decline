//! What the core hands the service over `invite.decline`, held in its configuration
//! directory: the media server key it switches an invitation off with, and the table of
//! invitations it may act on.
//!
//! Each is written whole, by renaming a written copy over it, so a refusal or a pass over
//! lapsed invitations never reads half of one, and readable by the service's own user
//! alone.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use lemonfiber_ports::service::{Claimed, Declining, Failure, Invitations};
use lemonfiber_sidecar::decline::{Claimed as Telling, File, Invitation, Key, Table, TokenHash};

/// The name the service gives itself in a failure.
const SERVICE: &str = "decline";

/// How many lower-case hexadecimal digits a token's SHA-256 is written in.
const HASH_DIGITS: usize = 64;

/// Who may read what it holds: the service's own user alone.
const OWNER_ONLY: u32 = 0o600;

/// The key and the table, held in the configuration directory `config`, written one at a
/// time.
pub(crate) struct Holding {
    config: PathBuf,
    writing: tokio::sync::Mutex<()>,
}

impl Holding {
    /// What is held in `config`.
    pub(crate) fn in_directory(config: &Path) -> Self {
        Self {
            config: config.to_path_buf(),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    /// Write `text` to `file`, whole, from a copy made afresh, so a copy left behind lends
    /// it neither its permissions nor a link.
    async fn written(&self, file: File, text: &str) -> Result<(), Failure> {
        let _writing = self.writing.lock().await;
        let record = self.config.join(file.name());
        let writing = self.config.join(format!("{}.writing", file.name()));
        let text = text.to_owned();
        let done = tokio::task::spawn_blocking(move || {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            match std::fs::remove_file(&writing) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(OWNER_ONLY)
                .open(&writing)?;
            out.write_all(text.as_bytes())?;
            out.sync_all()?;
            std::fs::rename(&writing, &record)
        })
        .await;
        done.ok()
            .and_then(Result::ok)
            .ok_or_else(|| refused(&format!("{} could not be written", file.name())))
    }
}

#[async_trait]
impl Declining for Holding {
    async fn hold(&self, key: &str) -> Result<(), Failure> {
        let key = Key::read(key).map_err(|unreadable| refused(&unreadable.to_string()))?;
        self.written(File::Key, &key.written()).await
    }

    async fn held(&self) -> Result<Option<String>, Failure> {
        Ok(
            tokio::fs::read_to_string(self.config.join(File::Key.name()))
                .await
                .ok()
                .and_then(|text| Key::read(&text).ok())
                .map(|key| key.fingerprint()),
        )
    }

    async fn apply_invitations(&self, invitations: &Invitations) -> Result<(), Failure> {
        let table = table(invitations)?;
        self.written(File::Table, &table.written()).await
    }
}

/// `invitations` as the table the service acts on, or why one of them is refused.
fn table(invitations: &Invitations) -> Result<Table, Failure> {
    let rows = invitations
        .invitations
        .iter()
        .map(|one| {
            hashed(&one.token).then(|| Invitation {
                token: TokenHash::written(&one.token),
                account: one.account.clone(),
                name: one.name.clone(),
                issued: one.issued,
                lapses: one.lapses,
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            refused("an invitation's token is not a SHA-256 in lower-case hexadecimal")
        })?;
    Ok(Table::of(telling(invitations.claimed), rows))
}

/// How the table tells a claimed account, as the core says the media server does.
const fn telling(claimed: Claimed) -> Telling {
    match claimed {
        Claimed::HasPassword => Telling::HasPassword,
        Claimed::OwnWrites => Telling::OwnWrites,
        Claimed::Unknown => Telling::Unknown,
    }
}

/// Whether `token` is a SHA-256 in lower-case hexadecimal.
fn hashed(token: &str) -> bool {
    token.len() == HASH_DIGITS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A refusal saying `detail`.
fn refused(detail: &str) -> Failure {
    Failure::Refused {
        service: SERVICE.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests;
