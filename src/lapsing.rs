//! Taking an invitation back when its window closes.
//!
//! Once a minute the service reads the core's table for invitations whose lapse time
//! has passed, that nobody declined, and that it has not already taken back. An account
//! nobody was ever seen in is removed; one somebody has been in, which is a reset nobody
//! took up, is switched off and kept, because it holds what they watched.
//!
//! **Nothing is removed on doubt.** A removal needs every guard to hold on reads made
//! just before the call: the table names the account, it is no administrator, it has no
//! password, nobody was ever seen in it, and the table, read again, still names it under
//! the same issue time. Any other answer switches the account off or leaves it as it is,
//! and a read that does not answer is tried again on the next pass.

use std::future::Future;

use lemonfiber_sidecar::decline::{
    Claimed, Invitation, Lapse, Lapses, Left, Outcome, Refusals, Table,
};

use crate::jellyfin::{normalised, Account, Server};

/// Take back every invitation in `table` whose window had closed by `now`, and the
/// lapses as they then stand.
///
/// `reread` reads the table again; it is asked once per invitation, just before the
/// call that acts on it. An invitation a read did not answer for is not recorded, so
/// the next pass tries it again.
pub(crate) async fn swept<Reread, Read>(
    now: u64,
    table: &Table,
    refusals: &Refusals,
    mut lapses: Lapses,
    server: &impl Server,
    reread: Reread,
) -> Lapses
where
    Reread: Fn() -> Read,
    Read: Future<Output = Option<Table>>,
{
    for invitation in &table.invitations {
        if invitation.open_at(now)
            || refusals.of(&invitation.token).is_some()
            || lapses.of(&invitation.token).is_some()
        {
            continue;
        }
        if let Some(outcome) = taken_back(invitation, server, &reread).await {
            lapses = lapses.with(Lapse {
                token: invitation.token.clone(),
                account: invitation.account.clone(),
                name: invitation.name.clone(),
                issued: invitation.issued,
                at: now,
                outcome,
            });
        }
    }
    lapses
}

/// Take back the one invitation whose window has closed, and what came of it, or
/// nothing where a read or a write did not answer.
async fn taken_back<Reread, Read>(
    invitation: &Invitation,
    server: &impl Server,
    reread: &Reread,
) -> Option<Outcome>
where
    Reread: Fn() -> Read,
    Read: Future<Output = Option<Table>>,
{
    let account = match server.account(&invitation.account).await {
        Ok(Some(account)) => account,
        Ok(None) => return Some(Outcome::Left(Left::Gone)),
        Err(_) => return None,
    };
    if account.administrator {
        return Some(Outcome::Left(Left::Administrator));
    }
    if account.disabled {
        return Some(Outcome::Left(Left::Disabled));
    }
    match server
        .claimed_since(&invitation.account, invitation.issued)
        .await
    {
        Ok(false) => {}
        Ok(true) => return Some(Outcome::Left(Left::Claimed)),
        Err(_) => return None,
    }
    let fresh = reread().await?;
    if fresh.claimed == Claimed::HasPassword && account.has_password == Some(true) {
        return Some(Outcome::Left(Left::Claimed));
    }
    if !still_named(&fresh, invitation) {
        return Some(Outcome::Left(Left::Reoffered));
    }
    if account.id.as_deref().map(normalised) != Some(normalised(&invitation.account)) {
        return Some(Outcome::Left(Left::Unmatched));
    }
    if removable(&account, fresh.claimed) {
        server
            .remove(&invitation.account)
            .await
            .ok()
            .map(|()| Outcome::Removed)
    } else {
        server
            .disable(&invitation.account, account.policy)
            .await
            .ok()
            .map(|()| Outcome::SwitchedOff)
    }
}

/// Whether `fresh` still names the account `invitation` was made for, under the same
/// issue time and the same token.
fn still_named(fresh: &Table, invitation: &Invitation) -> bool {
    fresh
        .find(&invitation.token)
        .is_some_and(|row| row.account == invitation.account && row.issued == invitation.issued)
}

/// Whether an account, already known to be unclaimed since the issue time, switched on,
/// no administrator and the one the table names, may be removed: nobody was ever seen
/// in it, and it has no password by the line's own way of saying so.
fn removable(account: &Account, claimed: Claimed) -> bool {
    let passwordless = match claimed {
        Claimed::HasPassword => account.has_password == Some(false),
        Claimed::OwnWrites => true,
        Claimed::Unknown => false,
    };
    passwordless && !account.seen
}

#[cfg(test)]
mod tests;
