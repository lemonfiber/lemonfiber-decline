use std::sync::Mutex;

use lemonfiber_sidecar::decline::{
    Claimed, Invitation, Lapse, Lapses, Left, Outcome, Refusal, Refusals, Table, TokenHash,
};
use serde_json::{json, Value};

use super::swept;
use crate::jellyfin::{Account, Server, Silent};

/// A media server answering as told, and remembering every write it was asked for.
struct Fake {
    account: Result<Option<Account>, Silent>,
    claimed: Result<bool, Silent>,
    writing: Result<(), Silent>,
    written: Mutex<Vec<String>>,
}

impl Fake {
    /// An account nobody has claimed, used or administered: every guard holds.
    fn untouched() -> Self {
        Self {
            account: Ok(Some(Account {
                id: Some("8c7a".to_owned()),
                has_password: Some(false),
                seen: false,
                administrator: false,
                disabled: false,
                policy: json!({"IsDisabled": false}),
            })),
            claimed: Ok(false),
            writing: Ok(()),
            written: Mutex::new(Vec::new()),
        }
    }

    /// The same account with `change` made to it.
    fn with(change: impl FnOnce(&mut Account)) -> Self {
        let mut fake = Self::untouched();
        if let Ok(Some(account)) = fake.account.as_mut() {
            change(account);
        }
        fake
    }

    fn written(&self) -> Vec<String> {
        self.written
            .lock()
            .map(|one| one.clone())
            .unwrap_or_default()
    }
}

impl Server for Fake {
    async fn account(&self, _id: &str) -> Result<Option<Account>, Silent> {
        self.account.clone()
    }

    async fn claimed_since(&self, _id: &str, _since: u64) -> Result<bool, Silent> {
        self.claimed.clone()
    }

    async fn disable(&self, id: &str, _policy: Value) -> Result<(), Silent> {
        if let Ok(mut written) = self.written.lock() {
            written.push(format!("disable {id}"));
        }
        self.writing.clone()
    }

    async fn remove(&self, id: &str) -> Result<(), Silent> {
        if let Ok(mut written) = self.written.lock() {
            written.push(format!("remove {id}"));
        }
        self.writing.clone()
    }
}

fn invitation(token: &str, issued: u64) -> Invitation {
    Invitation {
        token: TokenHash::of(token),
        account: "8c7a".to_owned(),
        name: "Ana".to_owned(),
        issued,
        lapses: 2_000,
    }
}

fn table(claimed: Claimed) -> Table {
    Table {
        claimed,
        ..Table::of(vec![invitation("token", 1_000)])
    }
}

/// What one pass at `now` over `table` records, the table reading back as `fresh`.
async fn pass(
    now: u64,
    table: &Table,
    refusals: &Refusals,
    lapses: Lapses,
    server: &Fake,
    fresh: Option<Table>,
) -> Lapses {
    swept(now, table, refusals, lapses, server, || {
        let fresh = fresh.clone();
        async move { fresh }
    })
    .await
}

/// What a pass after the window closed records for the one invitation, on a table
/// naming `claimed` that reads back unchanged.
async fn outcome(server: &Fake, claimed: Claimed) -> Option<Outcome> {
    let table = table(claimed);
    let lapses = pass(
        2_000,
        &table,
        &Refusals::default(),
        Lapses::default(),
        server,
        Some(table.clone()),
    )
    .await;
    lapses.of(&TokenHash::of("token")).map(|one| one.outcome)
}

#[tokio::test]
async fn an_account_nobody_was_seen_in_is_removed_when_its_window_closes() {
    let server = Fake::untouched();

    let lapses = pass(
        2_000,
        &table(Claimed::HasPassword),
        &Refusals::default(),
        Lapses::default(),
        &server,
        Some(table(Claimed::HasPassword)),
    )
    .await;

    assert_eq!(
        lapses.lapses,
        vec![Lapse {
            token: TokenHash::of("token"),
            account: "8c7a".to_owned(),
            name: "Ana".to_owned(),
            issued: 1_000,
            at: 2_000,
            outcome: Outcome::Removed,
        }]
    );
    assert_eq!(server.written(), vec!["remove 8c7a".to_owned()]);
}

#[tokio::test]
async fn an_open_invitation_is_left_to_stand() {
    let server = Fake::untouched();

    let lapses = pass(
        1_999,
        &table(Claimed::HasPassword),
        &Refusals::default(),
        Lapses::default(),
        &server,
        Some(table(Claimed::HasPassword)),
    )
    .await;

    assert!(lapses.lapses.is_empty());
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_administrator_is_never_removed_or_switched_off() {
    let server = Fake::with(|account| account.administrator = true);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Administrator))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_account_with_a_password_is_left_as_claimed() {
    let server = Fake::with(|account| account.has_password = Some(true));

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Claimed))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_account_whose_password_moved_since_the_issue_time_is_left_as_claimed() {
    let mut server = Fake::untouched();
    server.claimed = Ok(true);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Claimed))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_account_that_does_not_say_whether_it_has_a_password_is_switched_off_not_removed() {
    let server = Fake::with(|account| account.has_password = None);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::SwitchedOff)
    );
    assert_eq!(server.written(), vec!["disable 8c7a".to_owned()]);
}

#[tokio::test]
async fn an_account_somebody_has_been_in_is_switched_off_and_kept() {
    let server = Fake::with(|account| account.seen = true);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::SwitchedOff)
    );
    assert_eq!(server.written(), vec!["disable 8c7a".to_owned()]);
}

#[tokio::test]
async fn an_account_already_switched_off_is_left_as_it_is() {
    let server = Fake::with(|account| account.disabled = true);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Disabled))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_account_the_server_answers_for_under_another_id_is_left_as_it_is() {
    let server = Fake::with(|account| account.id = Some("somebody-else".to_owned()));

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Unmatched))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_account_the_server_answers_for_with_no_id_is_left_as_it_is() {
    let server = Fake::with(|account| account.id = None);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Unmatched))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn an_id_written_with_hyphens_is_the_same_account() {
    let server = Fake::with(|account| account.id = Some("8C-7A".to_owned()));

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Removed)
    );
}

#[tokio::test]
async fn an_account_offered_again_since_the_table_was_read_is_left_as_it_is() {
    let reissued = Table::of(vec![invitation("token", 1_500)]);
    let withdrawn = Table::of(Vec::new());

    for fresh in [reissued, withdrawn] {
        let server = Fake::untouched();
        let lapses = pass(
            2_000,
            &table(Claimed::HasPassword),
            &Refusals::default(),
            Lapses::default(),
            &server,
            Some(fresh),
        )
        .await;

        assert_eq!(
            lapses.of(&TokenHash::of("token")).map(|one| one.outcome),
            Some(Outcome::Left(Left::Reoffered))
        );
        assert!(server.written().is_empty());
    }
}

#[tokio::test]
async fn a_table_that_cannot_be_read_again_acts_on_nothing_and_is_tried_again() {
    let server = Fake::untouched();

    let lapses = pass(
        2_000,
        &table(Claimed::HasPassword),
        &Refusals::default(),
        Lapses::default(),
        &server,
        None,
    )
    .await;

    assert!(lapses.lapses.is_empty());
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn a_server_that_does_not_answer_is_asked_again_on_the_next_pass() {
    let mut silent = Fake::untouched();
    silent.account = Err(Silent);
    let mut unsure = Fake::untouched();
    unsure.claimed = Err(Silent);
    let mut refusing = Fake::untouched();
    refusing.writing = Err(Silent);

    for server in [silent, unsure, refusing] {
        assert_eq!(outcome(&server, Claimed::HasPassword).await, None);
    }
}

#[tokio::test]
async fn an_account_the_server_no_longer_holds_is_recorded_as_gone() {
    let mut server = Fake::untouched();
    server.account = Ok(None);

    assert_eq!(
        outcome(&server, Claimed::HasPassword).await,
        Some(Outcome::Left(Left::Gone))
    );
    assert!(server.written().is_empty());
}

#[tokio::test]
async fn on_a_line_that_reports_every_account_as_having_a_password_own_writes_decides() {
    let server = Fake::with(|account| account.has_password = Some(true));

    assert_eq!(
        outcome(&server, Claimed::OwnWrites).await,
        Some(Outcome::Removed)
    );
}

#[tokio::test]
async fn under_a_strategy_this_build_does_not_know_nothing_is_removed() {
    let server = Fake::untouched();

    assert_eq!(
        outcome(&server, Claimed::Unknown).await,
        Some(Outcome::SwitchedOff)
    );
    assert_eq!(server.written(), vec!["disable 8c7a".to_owned()]);
}

#[tokio::test]
async fn a_declined_invitation_or_one_already_taken_back_is_not_taken_back() {
    let declined = Refusals::default().with(Refusal {
        token: TokenHash::of("token"),
        account: "8c7a".to_owned(),
        at: 1_500,
    });
    let earlier = Lapses::default().with(Lapse {
        token: TokenHash::of("token"),
        account: "8c7a".to_owned(),
        name: "Ana".to_owned(),
        issued: 1_000,
        at: 2_000,
        outcome: Outcome::SwitchedOff,
    });

    for (refusals, lapses, held) in [
        (declined, Lapses::default(), 0),
        (Refusals::default(), earlier, 1),
    ] {
        let server = Fake::untouched();
        let after = pass(
            3_000,
            &table(Claimed::HasPassword),
            &refusals,
            lapses,
            &server,
            Some(table(Claimed::HasPassword)),
        )
        .await;

        assert_eq!(after.lapses.len(), held);
        assert!(server.written().is_empty());
    }
}
