//! Where an invitation stands, and declining it.
//!
//! Reading where it stands needs the core's two files and nothing else, so showing
//! the page never uses the key. Declining reads the account, reads whether it was
//! claimed, writes its policy back disabled, and records the refusal, in that order:
//! the account cannot be signed in to from the moment the server accepts the policy.

use lemonfiber_sidecar::decline::{Invitation, Refusal, Refusals, Table, TokenHash};

use crate::jellyfin::Server;

/// Where an invitation stands, in the terms the page answers in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Open, made for the account called this.
    Open(String),
    /// Declined just now.
    Declined,
    /// Declined before.
    AlreadyDeclined,
    /// Unknown, or lapsed.
    NoLongerOpen,
    /// The account was claimed, so it is not declined here.
    Accepted,
    /// The media server did not answer, so nothing was declined.
    Silent,
    /// Anything else that refuses it: an administrator's account, or one the server
    /// will not describe.
    Refused,
}

/// Where the invitation `token` declines stands at `now`, from the core's table and
/// the refusals recorded so far.
pub(crate) fn standing(token: &str, now: u64, table: &Table, refusals: &Refusals) -> Standing {
    match open(&TokenHash::of(token), now, table, refusals) {
        Ok(invitation) => Standing::Open(invitation.name.clone()),
        Err(standing) => standing,
    }
}

/// The invitation `token` names where it is open at `now`, or where it stands instead.
fn open<'a>(
    token: &TokenHash,
    now: u64,
    table: &'a Table,
    refusals: &Refusals,
) -> Result<&'a Invitation, Standing> {
    if refusals.of(token).is_some() {
        return Err(Standing::AlreadyDeclined);
    }
    match table.find(token) {
        Some(invitation) if invitation.open_at(now) => Ok(invitation),
        _ => Err(Standing::NoLongerOpen),
    }
}

/// Decline the invitation `token` names at `now`, and the refusals as they then stand.
///
/// Refusals come back unchanged unless this declined it.
pub(crate) async fn decline(
    token: &str,
    now: u64,
    table: &Table,
    refusals: Refusals,
    server: &impl Server,
) -> (Standing, Refusals) {
    let hash = TokenHash::of(token);
    let invitation = match open(&hash, now, table, &refusals) {
        Ok(invitation) => invitation,
        Err(standing) => return (standing, refusals),
    };

    let account = match server.account(&invitation.account).await {
        Ok(Some(account)) => account,
        Ok(None) => return (Standing::NoLongerOpen, refusals),
        Err(_) => return (Standing::Silent, refusals),
    };
    if account.administrator {
        return (Standing::Refused, refusals);
    }
    match server
        .claimed_since(&invitation.account, invitation.issued)
        .await
    {
        Ok(false) => {}
        Ok(true) => return (Standing::Accepted, refusals),
        Err(_) => return (Standing::Silent, refusals),
    }
    if !account.disabled
        && server
            .disable(&invitation.account, account.policy)
            .await
            .is_err()
    {
        return (Standing::Silent, refusals);
    }

    let refusals = refusals.with(Refusal {
        token: hash,
        account: invitation.account.clone(),
        at: now,
    });
    (Standing::Declined, refusals)
}

#[cfg(test)]
mod tests;
