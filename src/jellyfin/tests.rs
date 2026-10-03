use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use lemonfiber_sidecar::decline::Key;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::{account, claimed, disabled, iso, seconds, Jellyfin, Server, Silent};

#[test]
fn an_account_is_read_from_its_policy() {
    let user =
        json!({"Id": "8c7a", "Policy": {"IsAdministrator": false, "IsDisabled": true, "X": 1}});

    let read = account(&user);

    assert_eq!(read.as_ref().map(|one| one.administrator), Some(false));
    assert_eq!(read.as_ref().map(|one| one.disabled), Some(true));
    assert_eq!(
        read.and_then(|one| one.policy.get("X").cloned()),
        Some(json!(1))
    );
}

#[test]
fn an_account_with_no_policy_cannot_be_read() {
    assert!(account(&json!({"Id": "8c7a"})).is_none());
    assert!(account(&json!({"Policy": {"IsDisabled": false}})).is_none());
}

#[test]
fn a_password_moved_after_the_invitation_is_a_claim() {
    let entries = json!({"Items": [
        {"Type": "UserPasswordChanged", "UserId": "8c7a-0001", "Date": "2026-10-03T10:00:00.1234567Z"}
    ]});
    let issued = seconds("2026-10-03T09:00:00Z").unwrap_or_default();

    assert!(claimed(&entries, "8C7A0001", issued));
}

#[test]
fn a_password_moved_before_the_invitation_or_on_another_account_is_not() {
    let entries = json!({"Items": [
        {"Type": "UserPasswordChanged", "UserId": "8c7a0001", "Date": "2026-10-03T08:00:00Z"},
        {"Type": "UserPasswordChanged", "UserId": "ffff", "Date": "2026-10-03T10:00:00Z"},
        {"Type": "UserCreated", "UserId": "8c7a0001", "Date": "2026-10-03T10:00:00Z"}
    ]});
    let issued = seconds("2026-10-03T09:00:00Z").unwrap_or_default();

    assert!(!claimed(&entries, "8c7a0001", issued));
    assert!(!claimed(&json!({}), "8c7a0001", issued));
}

#[test]
fn a_policy_is_written_back_whole_with_only_disabled_set() {
    let policy =
        disabled(json!({"IsDisabled": false, "IsAdministrator": false, "MaxParentalRating": 12}));

    assert_eq!(
        policy,
        json!({"IsDisabled": true, "IsAdministrator": false, "MaxParentalRating": 12})
    );
}

#[test]
fn dates_go_out_and_come_back_as_the_same_second() {
    assert_eq!(iso(1_790_812_800), "2026-10-01T00:00:00Z");
    assert_eq!(seconds("2026-10-01T00:00:00Z"), Some(1_790_812_800));
    assert_eq!(seconds("not a date"), None);
}

/// What the fake server was sent.
#[derive(Default)]
struct Heard {
    authorisation: Vec<String>,
    policies: Vec<Value>,
}

type Ear = Arc<Mutex<Heard>>;

async fn fake(status: StatusCode) -> (String, Ear) {
    let heard: Ear = Arc::default();
    let user = move |State(heard): State<Ear>, headers: HeaderMap, Path(id): Path<String>| async move {
        if let Ok(mut heard) = heard.lock() {
            heard.authorisation.push(
                headers
                    .get("Authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        match (status, id.as_str()) {
            (StatusCode::OK, "known") => (
                StatusCode::OK,
                Json(json!({"Policy": {"IsAdministrator": false}})),
            ),
            (StatusCode::OK, _) => (StatusCode::NOT_FOUND, Json(json!({}))),
            (other, _) => (other, Json(json!({}))),
        }
    };
    let policy = |State(heard): State<Ear>, Path(id): Path<String>, Json(body): Json<Value>| async move {
        if id == "refuses" {
            return StatusCode::FORBIDDEN;
        }
        if let Ok(mut heard) = heard.lock() {
            heard.policies.push(body);
        }
        StatusCode::NO_CONTENT
    };
    let entries = || async {
        Json(json!({"Items": [
            {"Type": "UserPasswordChanged", "UserId": "known", "Date": "2026-10-03T10:00:00Z"}
        ]}))
    };
    let app = Router::new()
        .route("/Users/{id}", get(user))
        .route("/Users/{id}/Policy", post(policy))
        .route("/System/ActivityLog/Entries", get(entries))
        .with_state(heard.clone());
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return (String::new(), heard);
    };
    let at = listener
        .local_addr()
        .map(|at| at.to_string())
        .unwrap_or_default();
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{at}"), heard)
}

fn jellyfin(base: String) -> Jellyfin {
    Jellyfin {
        base,
        key: Key::read("the-key").unwrap_or_else(|_| unreachable!("a key")),
        client: reqwest::Client::new(),
    }
}

#[tokio::test]
async fn every_call_carries_the_key_in_the_media_browser_header() {
    let (base, heard) = fake(StatusCode::OK).await;

    let _ = jellyfin(base).account("known").await;

    let said = heard
        .lock()
        .map(|heard| heard.authorisation.clone())
        .unwrap_or_default();
    assert!(said
        .iter()
        .all(|one| one.starts_with("MediaBrowser ") && one.contains("Token=\"the-key\"")));
    assert_eq!(said.len(), 1);
}

#[tokio::test]
async fn an_account_the_server_does_not_hold_is_none_and_a_failure_is_silent() {
    let (base, _) = fake(StatusCode::OK).await;
    assert_eq!(jellyfin(base.clone()).account("unknown").await, Ok(None));
    assert!(jellyfin(base)
        .account("known")
        .await
        .is_ok_and(|one| one.is_some()));

    let (failing, _) = fake(StatusCode::INTERNAL_SERVER_ERROR).await;
    assert_eq!(jellyfin(failing).account("known").await, Err(Silent));

    assert_eq!(
        jellyfin("http://127.0.0.1:9".to_owned())
            .account("known")
            .await,
        Err(Silent)
    );
}

#[tokio::test]
async fn the_activity_log_says_whether_the_account_was_claimed() {
    let (base, _) = fake(StatusCode::OK).await;
    let issued = seconds("2026-10-03T09:00:00Z").unwrap_or_default();

    assert_eq!(
        jellyfin(base.clone()).claimed_since("known", issued).await,
        Ok(true)
    );
    assert_eq!(
        jellyfin(base).claimed_since("other", issued).await,
        Ok(false)
    );
}

#[tokio::test]
async fn disabling_writes_the_policy_back_with_disabled_set() {
    let (base, heard) = fake(StatusCode::OK).await;

    let written = jellyfin(base)
        .disable("known", json!({"IsDisabled": false, "Kept": 3}))
        .await;

    assert_eq!(written, Ok(()));
    let sent = heard
        .lock()
        .map(|heard| heard.policies.clone())
        .unwrap_or_default();
    assert_eq!(sent, vec![json!({"IsDisabled": true, "Kept": 3})]);
}

#[tokio::test]
async fn a_policy_the_server_refuses_is_silent() {
    let (base, _) = fake(StatusCode::OK).await;
    assert_eq!(
        jellyfin(base).disable("refuses", json!({})).await,
        Err(Silent)
    );
    assert_eq!(
        jellyfin("http://127.0.0.1:9".to_owned())
            .disable("known", json!({}))
            .await,
        Err(Silent)
    );
}
