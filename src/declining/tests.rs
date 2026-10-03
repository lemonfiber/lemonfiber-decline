use std::sync::Mutex;

use lemonfiber_sidecar::decline::{Invitation, Refusal, Refusals, Table, TokenHash};
use serde_json::{json, Value};

use super::{decline, standing, Standing};
use crate::jellyfin::{Account, Server, Silent};

/// A media server answering as told, and remembering what it was asked to disable.
struct Fake {
    account: Result<Option<Account>, Silent>,
    claimed: Result<bool, Silent>,
    disabling: Result<(), Silent>,
    disabled: Mutex<Vec<String>>,
}

impl Fake {
    fn holding(administrator: bool, disabled: bool) -> Self {
        Self {
            account: Ok(Some(Account {
                administrator,
                disabled,
                policy: json!({"IsDisabled": disabled}),
            })),
            claimed: Ok(false),
            disabling: Ok(()),
            disabled: Mutex::new(Vec::new()),
        }
    }

    fn disabled(&self) -> Vec<String> {
        self.disabled
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
        if let Ok(mut disabled) = self.disabled.lock() {
            disabled.push(id.to_owned());
        }
        self.disabling.clone()
    }
}

fn table() -> Table {
    Table::of(vec![Invitation {
        token: TokenHash::of("token"),
        account: "8c7a".to_owned(),
        name: "Ana".to_owned(),
        issued: 1_000,
        lapses: 2_000,
    }])
}

#[test]
fn an_open_invitation_names_the_account_it_was_made_for() {
    assert_eq!(
        standing("token", 1_500, &table(), &Refusals::default()),
        Standing::Open("Ana".to_owned())
    );
}

#[test]
fn an_unknown_or_lapsed_token_is_no_longer_open() {
    assert_eq!(
        standing("other", 1_500, &table(), &Refusals::default()),
        Standing::NoLongerOpen
    );
    assert_eq!(
        standing("token", 2_000, &table(), &Refusals::default()),
        Standing::NoLongerOpen
    );
}

#[test]
fn a_declined_invitation_says_so_even_after_it_lapses() {
    let refusals = Refusals::default().with(Refusal {
        token: TokenHash::of("token"),
        account: "8c7a".to_owned(),
        at: 1_500,
    });

    assert_eq!(
        standing("token", 3_000, &table(), &refusals),
        Standing::AlreadyDeclined
    );
}

#[tokio::test]
async fn declining_disables_the_account_and_records_the_refusal() {
    let server = Fake::holding(false, false);

    let (said, refusals) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(said, Standing::Declined);
    assert_eq!(server.disabled(), vec!["8c7a".to_owned()]);
    assert_eq!(
        refusals.of(&TokenHash::of("token")).map(|one| one.at),
        Some(1_500)
    );
}

#[tokio::test]
async fn an_account_already_disabled_is_recorded_without_writing_it_again() {
    let server = Fake::holding(false, true);

    let (said, refusals) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(said, Standing::Declined);
    assert!(server.disabled().is_empty());
    assert_eq!(refusals.refusals.len(), 1);
}

#[tokio::test]
async fn a_second_refusal_writes_nothing() {
    let server = Fake::holding(false, false);
    let (_, once) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    let (said, twice) = decline("token", 1_600, &table(), once.clone(), &server).await;

    assert_eq!(said, Standing::AlreadyDeclined);
    assert_eq!(twice, once);
    assert_eq!(server.disabled().len(), 1);
}

#[tokio::test]
async fn an_administrators_account_is_refused_and_left_alone() {
    let server = Fake::holding(true, false);

    let (said, refusals) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(said, Standing::Refused);
    assert!(server.disabled().is_empty());
    assert!(refusals.refusals.is_empty());
}

#[tokio::test]
async fn a_claimed_account_is_accepted_and_left_alone() {
    let server = Fake {
        claimed: Ok(true),
        ..Fake::holding(false, false)
    };

    let (said, _) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(said, Standing::Accepted);
    assert!(server.disabled().is_empty());
}

#[tokio::test]
async fn an_account_the_server_no_longer_holds_is_no_longer_open() {
    let server = Fake {
        account: Ok(None),
        ..Fake::holding(false, false)
    };

    let (said, _) = decline("token", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(said, Standing::NoLongerOpen);
}

#[tokio::test]
async fn a_silent_server_declines_nothing_at_any_step() {
    for server in [
        Fake {
            account: Err(Silent),
            ..Fake::holding(false, false)
        },
        Fake {
            claimed: Err(Silent),
            ..Fake::holding(false, false)
        },
        Fake {
            disabling: Err(Silent),
            ..Fake::holding(false, false)
        },
    ] {
        let (said, refusals) =
            decline("token", 1_500, &table(), Refusals::default(), &server).await;

        assert_eq!(said, Standing::Silent);
        assert!(refusals.refusals.is_empty());
    }
}

#[tokio::test]
async fn a_lapsed_or_unknown_token_asks_the_server_nothing() {
    let server = Fake::holding(false, false);

    let (lapsed, _) = decline("token", 2_500, &table(), Refusals::default(), &server).await;
    let (unknown, _) = decline("other", 1_500, &table(), Refusals::default(), &server).await;

    assert_eq!(
        (lapsed, unknown),
        (Standing::NoLongerOpen, Standing::NoLongerOpen)
    );
    assert!(server.disabled().is_empty());
}
