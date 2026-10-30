//! What the core hands the service over `invite.decline`, held in its configuration
//! directory: the media server key it switches an invitation off with, the certificates
//! the media server may present to it, and the table of invitations it may act on.
//!
//! Each is written whole, by renaming a written copy over it, so a refusal or a pass over
//! lapsed invitations never reads half of one, and readable by the service's own user
//! alone. What it declined and what it took back as each window closed are read back
//! from the records it keeps beside them.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use lemonfiber_ports::service::{
    Claimed, Declined, Declining, Failure, Invitations, KeyHeld, Lapsed, LeftBecause, TakenBack,
};
use lemonfiber_sidecar::decline::{
    Claimed as Telling, File, Invitation, Key, Lapse, Lapses, Left, Outcome, Refusal, Refusals,
    Table, TokenHash,
};
use lemonfiber_sidecar::Pins;

/// The name the service gives itself in a failure.
const SERVICE: &str = "decline";

/// How many lower-case hexadecimal digits a token's SHA-256 is written in.
const HASH_DIGITS: usize = 64;

/// Who may read what it holds: the service's own user alone.
const OWNER_ONLY: u32 = 0o600;

/// The key, the pins and the table, held in the configuration directory `config`, written
/// one at a time.
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

    /// Write `text` to the file `name`, whole, one write at a time.
    async fn written(&self, name: &str, text: &str) -> Result<(), Failure> {
        let _writing = self.writing.lock().await;
        replaced(&self.config, name, text)
            .await
            .map_err(|_| refused(&format!("{name} could not be written")))
    }

    /// What `file` holds, none where it is not there, or why it cannot be read.
    async fn kept(&self, file: File) -> Result<Option<String>, Failure> {
        match tokio::fs::read_to_string(self.config.join(file.name())).await {
            Ok(text) => Ok(Some(text)),
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(unreadable(file)),
        }
    }
}

#[async_trait]
impl Declining for Holding {
    async fn hold(&self, key: &str, pins: &[String]) -> Result<(), Failure> {
        let key = Key::read(key).map_err(|unreadable| refused(&unreadable.to_string()))?;
        let pins = Pins::of(pins).map_err(|unreadable| refused(&unreadable.to_string()))?;
        self.written(File::Pins.name(), &pins.written()).await?;
        self.written(File::Key.name(), &key.written()).await
    }

    async fn held(&self) -> Result<Option<KeyHeld>, Failure> {
        let Some(key) = tokio::fs::read_to_string(self.config.join(File::Key.name()))
            .await
            .ok()
            .and_then(|text| Key::read(&text).ok())
        else {
            return Ok(None);
        };
        let pins = tokio::fs::read_to_string(self.config.join(File::Pins.name()))
            .await
            .ok()
            .and_then(|text| Pins::read(&text).ok())
            .map(Vec::from)
            .unwrap_or_default();
        Ok(Some(KeyHeld {
            key: key.fingerprint(),
            pins,
        }))
    }

    async fn apply_invitations(&self, invitations: &Invitations) -> Result<(), Failure> {
        let table = table(invitations)?;
        self.written(File::Table.name(), &table.written()).await
    }

    async fn refusals(&self) -> Result<Vec<Declined>, Failure> {
        let refusals = self
            .kept(File::Refusals)
            .await?
            .map(|text| Refusals::read(&text))
            .transpose()
            .map_err(|_| unreadable(File::Refusals))?
            .unwrap_or_default();
        Ok(refusals.refusals.iter().map(declined).collect())
    }

    async fn lapses(&self) -> Result<Vec<Lapsed>, Failure> {
        let lapses = self
            .kept(File::Lapses)
            .await?
            .map(|text| Lapses::read(&text))
            .transpose()
            .map_err(|_| unreadable(File::Lapses))?
            .unwrap_or_default();
        Ok(lapses.lapses.iter().map(lapsed).collect())
    }

    async fn page(&self) -> Result<String, Failure> {
        Ok(crate::serving::PAGE.to_owned())
    }
}

/// Write `text` to the file `name` in `config`, whole, from a copy made afresh and
/// readable by the service's own user alone, so a copy left behind lends it neither its
/// permissions nor a link, and a reader never sees half of one.
///
/// # Errors
///
/// Why the copy could not be cleared, written or renamed over the file.
pub(crate) async fn replaced(config: &Path, name: &str, text: &str) -> std::io::Result<()> {
    let record = config.join(name);
    let writing = config.join(format!("{name}.writing"));
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || {
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
    .await
    .map_err(std::io::Error::other)?
}

/// `refusal` as the contract carries it.
fn declined(refusal: &Refusal) -> Declined {
    Declined {
        token: refusal.token.as_str().to_owned(),
        account: refusal.account.clone(),
        at: refusal.at,
    }
}

/// `lapse` as the contract carries it.
fn lapsed(lapse: &Lapse) -> Lapsed {
    Lapsed {
        token: lapse.token.as_str().to_owned(),
        account: lapse.account.clone(),
        name: lapse.name.clone(),
        issued: lapse.issued,
        at: lapse.at,
        outcome: match lapse.outcome {
            Outcome::Removed => TakenBack::Removed,
            Outcome::SwitchedOff => TakenBack::SwitchedOff,
            Outcome::Left(why) => TakenBack::from(left(why)),
        },
    }
}

/// Why an account was left as it was, as the contract says it.
const fn left(why: Left) -> LeftBecause {
    match why {
        Left::Claimed => LeftBecause::Claimed,
        Left::Administrator => LeftBecause::Administrator,
        Left::Disabled => LeftBecause::Disabled,
        Left::Gone => LeftBecause::Gone,
        Left::Reoffered => LeftBecause::Reoffered,
        Left::Unmatched => LeftBecause::Unmatched,
    }
}

/// A record in `file` that could not be read.
fn unreadable(file: File) -> Failure {
    refused(&format!("{} could not be read", file.name()))
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
