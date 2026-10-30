use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use lemonfiber_ports::service::{
    Claimed, Declinable, Declined, Declining, Failure, Invitations, KeyHeld, Lapsed, LeftBecause,
    TakenBack,
};
use lemonfiber_sidecar::decline::{
    Claimed as Telling, File, Key, Lapse, Lapses, Left, Outcome, Refusal, Refusals, Table,
    TokenHash,
};

use super::Holding;

struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("holding-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn holding(&self) -> Holding {
        Holding::in_directory(&self.0)
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    fn mode(&self, file: File) -> u32 {
        std::fs::metadata(self.0.join(file.name()))
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or_default()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn a_key() -> String {
    ('g'..='z').collect()
}

fn pins() -> Vec<String> {
    vec!["ab".repeat(32), "cd".repeat(32)]
}

fn pins_file(config: &Config) -> String {
    std::fs::read_to_string(
        config
            .0
            .join(lemonfiber_sidecar::decline::File::Pins.name()),
    )
    .unwrap_or_default()
}

fn invitation(token: &str) -> Declinable {
    Declinable {
        token: TokenHash::of(token).as_str().to_owned(),
        account: "4f1c".to_owned(),
        name: "Ana".to_owned(),
        issued: 1_000,
        lapses: 2_000,
    }
}

#[tokio::test]
async fn a_held_key_is_answered_by_its_sha256_and_never_itself() {
    let config = Config::new("held");
    let holding = config.holding();

    assert_eq!(holding.held().await.ok(), Some(None));
    assert!(holding.hold(&a_key(), &pins()).await.is_ok());

    let held = holding.held().await.ok().flatten();
    assert_eq!(
        held,
        Some(KeyHeld {
            key: TokenHash::of(&a_key()).as_str().to_owned(),
            pins: pins(),
        })
    );
    assert!(held.is_some_and(|held| held.key != a_key()));
    assert_eq!(config.read(File::Key), format!("{}\n", a_key()));
    assert_eq!(
        pins_file(&config),
        format!("{}\n{}\n", "ab".repeat(32), "cd".repeat(32))
    );
}

#[tokio::test]
async fn the_key_and_the_table_are_readable_by_the_service_alone() {
    let config = Config::new("modes");
    let holding = config.holding();

    assert!(holding.hold(&a_key(), &pins()).await.is_ok());
    assert!(holding
        .apply_invitations(&Invitations {
            claimed: Claimed::HasPassword,
            invitations: vec![invitation("token")],
        })
        .await
        .is_ok());

    assert_eq!(config.mode(File::Key), 0o600);
    assert_eq!(config.mode(File::Table), 0o600);
    let pinned = std::fs::metadata(
        config
            .0
            .join(lemonfiber_sidecar::decline::File::Pins.name()),
    )
    .map(|meta| meta.permissions().mode() & 0o777)
    .unwrap_or_default();
    assert_eq!(pinned, 0o600);
}

#[tokio::test]
async fn a_copy_left_behind_lends_the_key_neither_its_permissions_nor_its_text() {
    let config = Config::new("left-behind");
    let left = config.0.join(format!("{}.writing", File::Key.name()));
    let _ = std::fs::write(&left, "stale stale stale");
    let _ = std::fs::set_permissions(&left, std::fs::Permissions::from_mode(0o666));

    assert!(config.holding().hold(&a_key(), &pins()).await.is_ok());

    assert_eq!(config.mode(File::Key), 0o600);
    assert_eq!(config.read(File::Key), format!("{}\n", a_key()));
}

#[tokio::test]
async fn a_new_key_takes_the_place_of_the_one_held() {
    let config = Config::new("replaced");
    let holding = config.holding();

    assert!(holding.hold("first", &pins()).await.is_ok());
    assert!(holding.hold(&a_key(), &pins()).await.is_ok());

    assert_eq!(
        Key::read(&config.read(File::Key))
            .ok()
            .map(|key| key.fingerprint()),
        Some(TokenHash::of(&a_key()).as_str().to_owned())
    );
}

#[tokio::test]
async fn a_key_that_is_empty_or_more_than_one_word_is_refused_and_nothing_is_written() {
    let config = Config::new("refused-key");
    let holding = config.holding();

    for key in ["", "  ", "two words"] {
        assert!(matches!(
            holding.hold(key, &pins()).await,
            Err(Failure::Refused { .. })
        ));
    }
    assert_eq!(config.read(File::Key), "");
}

#[tokio::test]
async fn a_pin_that_is_not_a_lower_case_sha256_is_refused_and_nothing_is_written() {
    let config = Config::new("refused-pin");
    let holding = config.holding();

    for pin in ["AB".repeat(32), "ab".repeat(31), "not a pin".to_owned()] {
        assert!(matches!(
            holding.hold(&a_key(), &[pin]).await,
            Err(Failure::Refused { .. })
        ));
    }
    assert_eq!(config.read(File::Key), "");
    assert_eq!(pins_file(&config), "");
}

#[tokio::test]
async fn a_key_held_with_no_pin_is_held_with_none() {
    let config = Config::new("no-pin");
    let holding = config.holding();

    assert!(holding.hold(&a_key(), &[]).await.is_ok());

    assert_eq!(
        holding.held().await.ok().flatten().map(|held| held.pins),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn the_invitations_applied_are_the_table_the_service_acts_on() {
    let config = Config::new("applied");
    let holding = config.holding();

    for (claimed, telling) in [
        (Claimed::HasPassword, Telling::HasPassword),
        (Claimed::OwnWrites, Telling::OwnWrites),
        (Claimed::Unknown, Telling::Unknown),
    ] {
        assert!(holding
            .apply_invitations(&Invitations {
                claimed,
                invitations: vec![invitation("token")],
            })
            .await
            .is_ok());

        let table = Table::read(&config.read(File::Table)).ok();
        assert_eq!(table.as_ref().map(|table| table.claimed), Some(telling));
        let found = table
            .as_ref()
            .and_then(|table| table.find(&TokenHash::of("token")));
        assert_eq!(found.map(|one| one.account.as_str()), Some("4f1c"));
        assert_eq!(found.map(|one| one.name.as_str()), Some("Ana"));
        assert_eq!(
            found.map(|one| (one.issued, one.lapses)),
            Some((1_000, 2_000))
        );
    }
}

#[tokio::test]
async fn invitations_applied_again_leave_none_of_the_earlier_ones() {
    let config = Config::new("again");
    let holding = config.holding();

    for token in ["first", "second"] {
        assert!(holding
            .apply_invitations(&Invitations {
                claimed: Claimed::HasPassword,
                invitations: vec![invitation(token)],
            })
            .await
            .is_ok());
    }

    let table = Table::read(&config.read(File::Table)).ok();
    assert_eq!(table.as_ref().map(|table| table.invitations.len()), Some(1));
    assert!(table
        .as_ref()
        .and_then(|table| table.find(&TokenHash::of("first")))
        .is_none());
}

#[tokio::test]
async fn an_invitation_whose_token_is_no_sha256_is_refused_and_the_table_kept() {
    let config = Config::new("refused-table");
    let holding = config.holding();
    assert!(holding
        .apply_invitations(&Invitations {
            claimed: Claimed::HasPassword,
            invitations: vec![invitation("token")],
        })
        .await
        .is_ok());
    let before = config.read(File::Table);

    let hash = TokenHash::of("token").as_str().to_owned();
    for token in [
        "the-token-itself".to_owned(),
        hash.to_ascii_uppercase(),
        hash[1..].to_owned(),
        format!("{hash}0"),
    ] {
        let mut one = invitation("token");
        one.token = token;
        assert!(matches!(
            holding
                .apply_invitations(&Invitations {
                    claimed: Claimed::HasPassword,
                    invitations: vec![invitation("other"), one],
                })
                .await,
            Err(Failure::Refused { .. })
        ));
    }
    assert_eq!(config.read(File::Table), before);
}

#[tokio::test]
async fn what_cannot_be_written_is_refused() {
    let config = Config::new("unwritable");
    let holding = Holding::in_directory(&config.0.join("missing"));

    assert!(matches!(
        holding.hold(&a_key(), &pins()).await,
        Err(Failure::Refused { .. })
    ));
    assert!(matches!(
        holding
            .apply_invitations(&Invitations {
                claimed: Claimed::HasPassword,
                invitations: vec![invitation("token")],
            })
            .await,
        Err(Failure::Refused { .. })
    ));
}

fn lapse(token: &str, outcome: Outcome) -> Lapse {
    Lapse {
        token: TokenHash::of(token),
        account: format!("account-{token}"),
        name: format!("name-{token}"),
        issued: 10,
        at: 20,
        outcome,
    }
}

#[tokio::test]
async fn every_refusal_recorded_is_answered_by_its_token_s_sha256_alone() {
    let config = Config::new("refusals");
    let refusals = Refusals::default()
        .with(Refusal {
            token: TokenHash::of("first"),
            account: "one".to_owned(),
            at: 5,
        })
        .with(Refusal {
            token: TokenHash::of("second"),
            account: "two".to_owned(),
            at: 6,
        });
    let _ = std::fs::write(config.0.join(File::Refusals.name()), refusals.written());

    let answer = config.holding().refusals().await;

    assert_eq!(
        answer.ok(),
        Some(vec![
            Declined {
                token: TokenHash::of("first").as_str().to_owned(),
                account: "one".to_owned(),
                at: 5,
            },
            Declined {
                token: TokenHash::of("second").as_str().to_owned(),
                account: "two".to_owned(),
                at: 6,
            },
        ])
    );
}

#[tokio::test]
async fn every_lapse_recorded_is_answered_with_what_became_of_its_account() {
    let config = Config::new("lapses");
    let outcomes = [
        (Outcome::Removed, TakenBack::Removed),
        (Outcome::SwitchedOff, TakenBack::SwitchedOff),
        (Outcome::Left(Left::Claimed), LeftBecause::Claimed.into()),
        (
            Outcome::Left(Left::Administrator),
            LeftBecause::Administrator.into(),
        ),
        (Outcome::Left(Left::Disabled), LeftBecause::Disabled.into()),
        (Outcome::Left(Left::Gone), LeftBecause::Gone.into()),
        (
            Outcome::Left(Left::Reoffered),
            LeftBecause::Reoffered.into(),
        ),
        (
            Outcome::Left(Left::Unmatched),
            LeftBecause::Unmatched.into(),
        ),
    ];
    let lapses = outcomes
        .iter()
        .enumerate()
        .fold(Lapses::default(), |lapses, (at, (outcome, _))| {
            lapses.with(lapse(&at.to_string(), *outcome))
        });
    let _ = std::fs::write(config.0.join(File::Lapses.name()), lapses.written());

    let answer = config.holding().lapses().await.unwrap_or_default();

    assert_eq!(
        answer,
        outcomes
            .iter()
            .enumerate()
            .map(|(at, (_, outcome))| Lapsed {
                token: TokenHash::of(&at.to_string()).as_str().to_owned(),
                account: format!("account-{at}"),
                name: format!("name-{at}"),
                issued: 10,
                at: 20,
                outcome: *outcome,
            })
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn nothing_recorded_yet_is_answered_as_nothing() {
    let config = Config::new("unrecorded");
    let holding = config.holding();

    assert_eq!(holding.refusals().await.ok(), Some(Vec::new()));
    assert_eq!(holding.lapses().await.ok(), Some(Vec::new()));
}

#[tokio::test]
async fn a_record_that_cannot_be_read_is_refused_by_name() {
    let config = Config::new("unreadable-records");
    let holding = config.holding();
    let _ = std::fs::write(config.0.join(File::Refusals.name()), "not a record");
    let _ = std::fs::create_dir(config.0.join(File::Lapses.name()));

    let refusals = holding.refusals().await;
    let lapses = holding.lapses().await;

    assert!(
        matches!(&refusals, Err(Failure::Refused { detail, .. }) if detail.contains(File::Refusals.name())),
        "{refusals:?}"
    );
    assert!(
        matches!(&lapses, Err(Failure::Refused { detail, .. }) if detail.contains(File::Lapses.name())),
        "{lapses:?}"
    );
    let _ = std::fs::remove_dir(config.0.join(File::Lapses.name()));
    let _ = std::fs::write(config.0.join(File::Lapses.name()), "not a record");
    assert!(matches!(
        holding.lapses().await,
        Err(Failure::Refused { .. })
    ));
}

#[tokio::test]
async fn the_page_is_answered_at_the_path_the_service_serves_it_under() {
    let config = Config::new("page");

    assert_eq!(
        config.holding().page().await.ok().as_deref(),
        Some("/decline/{token}")
    );
}
