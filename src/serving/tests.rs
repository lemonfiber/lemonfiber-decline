use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use lemonfiber_sidecar::decline::{
    File, Invitation, Key, Lapses, Outcome, Refusals, Table, TokenHash,
};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::{answers_ok, now, passed, routes, Service};
use crate::limit::Limit;
use crate::settings::Settings;

/// A configuration directory of its own, removed when dropped.
struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("decline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn write(&self, file: File, text: &str) {
        let _ = std::fs::write(self.0.join(file.name()), text);
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    fn with_invitation(self) -> Self {
        let table = Table::of(vec![Invitation {
            token: TokenHash::of("token"),
            account: "known".to_owned(),
            name: "Ana".to_owned(),
            issued: now() - 60,
            lapses: now() + 3_600,
        }]);
        self.write(File::Table, &table.written());
        self
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn service(config: &Config, jellyfin: &str, limit: Limit) -> Arc<Service> {
    Service::new(
        Settings {
            config: config.0.clone(),
            jellyfin: jellyfin.to_owned(),
        },
        limit,
    )
}

async fn asked(service: Arc<Service>, method: &str, path: &str) -> (StatusCode, String, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap_or_default();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((
            Ipv4Addr::new(192, 168, 1, 20),
            50_000,
        ))));
    let Ok(response) = routes(service).oneshot(request).await;
    let status = response.status();
    let policy = response
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&body).into_owned(), policy)
}

/// A media server that holds the account `known`, unclaimed, and takes its policy.
async fn jellyfin() -> String {
    let app = Router::new()
        .route(
            "/Users/{id}",
            get(|| async {
                Json(json!({"Policy": {"IsAdministrator": false, "IsDisabled": false}}))
            }),
        )
        .route(
            "/System/ActivityLog/Entries",
            get(|| async { Json(json!({"Items": []})) }),
        )
        .route(
            "/Users/{id}/Policy",
            post(|| async { StatusCode::NO_CONTENT }),
        );
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return String::new();
    };
    let at = listener
        .local_addr()
        .map(|at| at.to_string())
        .unwrap_or_default();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{at}")
}

#[tokio::test]
async fn health_says_which_key_it_holds_or_that_it_holds_none() {
    let config = Config::new("health");
    let (status, body, _) = asked(service(&config, "", Limit::standard()), "GET", "/health").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "{\"key\":null}"));

    config.write(File::Key, "the-key\n");
    let fingerprint = Key::read("the-key")
        .map(|key| key.fingerprint())
        .unwrap_or_default();
    let (_, body, _) = asked(service(&config, "", Limit::standard()), "GET", "/health").await;
    assert_eq!(body, format!("{{\"key\":\"{fingerprint}\"}}"));
    assert!(!body.contains("the-key"));
}

#[tokio::test]
async fn the_page_shows_an_open_invitation_and_locks_itself_down() {
    let config = Config::new("page").with_invitation();

    let (status, body, policy) = asked(
        service(&config, "", Limit::standard()),
        "GET",
        "/decline/token",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("as Ana."));
    assert!(policy.contains("default-src 'none'") && policy.contains("frame-ancestors 'none'"));
}

#[tokio::test]
async fn with_no_table_written_nothing_is_open() {
    let config = Config::new("empty");

    let (_, body, _) = asked(
        service(&config, "", Limit::standard()),
        "GET",
        "/decline/token",
    )
    .await;

    assert!(body.contains("This invitation is no longer open."));
}

#[tokio::test]
async fn a_refusal_disables_the_account_and_is_recorded_for_the_core() {
    let config = Config::new("refusal").with_invitation();
    config.write(File::Key, "the-key\n");
    let server = jellyfin().await;

    let (status, body, _) = asked(
        service(&config, &server, Limit::standard()),
        "POST",
        "/decline/token",
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("You declined the invitation."));
    let recorded = Refusals::read(&config.read(File::Refusals));
    assert!(recorded.is_ok_and(|refusals| refusals.of(&TokenHash::of("token")).is_some()));

    let (_, again, _) = asked(
        service(&config, &server, Limit::standard()),
        "GET",
        "/decline/token",
    )
    .await;
    assert!(again.contains("This invitation was already declined."));
}

#[tokio::test]
async fn without_its_key_the_service_declines_nothing() {
    let config = Config::new("keyless").with_invitation();

    let (_, body, _) = asked(
        service(&config, "", Limit::standard()),
        "POST",
        "/decline/token",
    )
    .await;

    assert!(body.contains("This invitation cannot be declined here."));
    assert!(config.read(File::Refusals).is_empty());
}

#[tokio::test]
async fn a_refusal_over_the_limit_is_turned_away() {
    let config = Config::new("limited").with_invitation();
    let limited = service(
        &config,
        "",
        Limit {
            asks: 1,
            window: 60,
        },
    );

    let _ = asked(limited.clone(), "POST", "/decline/other").await;
    let (status, body, _) = asked(limited, "POST", "/decline/other").await;

    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("Try again in a minute."));
}

#[tokio::test]
async fn anything_else_is_not_found() {
    let config = Config::new("other");
    let (status, _, _) = asked(service(&config, "", Limit::standard()), "GET", "/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_health_check_reads_a_serving_service_as_healthy() {
    let config = Config::new("serving");
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    let app = routes(service(&config, "", Limit::standard()));
    tokio::spawn(async move { axum::serve(listener, app).await });

    assert!(answers_ok(at).await);
}

#[tokio::test]
async fn the_health_check_reads_nothing_listening_as_unhealthy() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    drop(listener);

    assert!(!answers_ok(at).await);
}

#[tokio::test]
async fn serving_stops_when_told_to() {
    let config = Config::new("stops");
    let at = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));

    let stopped = super::serve(
        at,
        service(&config, "", Limit::standard()),
        std::future::ready(()),
    )
    .await;

    assert_eq!(stopped, std::process::ExitCode::SUCCESS);
}

#[tokio::test]
async fn an_address_already_taken_is_a_failure_to_serve() {
    let config = Config::new("taken");
    let Ok(taken) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = taken.local_addr() else {
        return;
    };

    let refused = super::serve(
        at,
        service(&config, "", Limit::standard()),
        std::future::ready(()),
    )
    .await;

    assert_eq!(refused, std::process::ExitCode::FAILURE);
}

#[tokio::test]
async fn the_health_command_answers_for_the_port_it_is_given() {
    let config = Config::new("command");
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    let app = routes(service(&config, "", Limit::standard()));
    tokio::spawn(async move { axum::serve(listener, app).await });

    assert_eq!(
        super::healthy(at.port()).await,
        std::process::ExitCode::SUCCESS
    );
}

#[tokio::test]
async fn the_health_command_fails_where_nothing_answers() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    drop(listener);

    assert_eq!(
        super::healthy(at.port()).await,
        std::process::ExitCode::FAILURE
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_refusal_that_cannot_be_recorded_still_says_what_was_done() {
    use std::os::unix::fs::PermissionsExt;

    let config = Config::new("unrecorded").with_invitation();
    config.write(File::Key, "the-key\n");
    let server = jellyfin().await;
    let _ = std::fs::set_permissions(&config.0, std::fs::Permissions::from_mode(0o555));

    let (_, body, _) = asked(
        service(&config, &server, Limit::standard()),
        "POST",
        "/decline/token",
    )
    .await;
    let _ = std::fs::set_permissions(&config.0, std::fs::Permissions::from_mode(0o755));

    assert!(body.contains("You declined the invitation."));
    assert!(config.read(File::Refusals).is_empty());
}

/// A media server that holds the account `known`, never signed in to, with no password,
/// and removes it when asked.
async fn jellyfin_holding_an_offer() -> String {
    let app = Router::new()
        .route(
            "/Users/{id}",
            get(|| async {
                Json(json!({"Id": "known", "HasPassword": false,
                    "Policy": {"IsAdministrator": false, "IsDisabled": false}}))
            })
            .merge(delete(|| async { StatusCode::NO_CONTENT })),
        )
        .route(
            "/System/ActivityLog/Entries",
            get(|| async { Json(json!({"Items": []})) }),
        );
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return String::new();
    };
    let at = listener
        .local_addr()
        .map(|at| at.to_string())
        .unwrap_or_default();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{at}")
}

/// A configuration directory holding a key and one invitation whose window closed a
/// minute ago.
fn lapsed(name: &str) -> Config {
    let config = Config::new(name);
    let table = Table::of(vec![Invitation {
        token: TokenHash::of("token"),
        account: "known".to_owned(),
        name: "Ana".to_owned(),
        issued: now() - 3_600,
        lapses: now() - 60,
    }]);
    config.write(File::Table, &table.written());
    config.write(File::Key, "the-key\n");
    config
}

#[tokio::test]
async fn a_pass_takes_back_a_lapsed_invitation_and_records_it_for_the_core() {
    let config = lapsed("lapse-pass");
    let server = jellyfin_holding_an_offer().await;

    let written = passed(&service(&config, &server, Limit::standard()), now()).await;

    let lapses = Lapses::read(&config.read(File::Lapses));
    assert!(written);
    assert_eq!(
        lapses
            .ok()
            .and_then(|lapses| lapses.of(&TokenHash::of("token")).map(|one| one.outcome)),
        Some(Outcome::Removed)
    );
}

#[tokio::test]
async fn a_pass_with_nothing_due_reaches_nothing_and_writes_nothing() {
    let config = Config::new("lapse-nothing").with_invitation();
    config.write(File::Key, "the-key\n");

    let written = passed(
        &service(&config, "http://127.0.0.1:9", Limit::standard()),
        now(),
    )
    .await;

    assert!(written);
    assert!(config.read(File::Lapses).is_empty());
}

#[tokio::test]
async fn a_record_of_lapses_that_cannot_be_read_stops_the_pass_and_is_kept() {
    let config = lapsed("lapse-garbled");
    config.write(File::Lapses, "not a record");
    let server = jellyfin_holding_an_offer().await;

    let _ = passed(&service(&config, &server, Limit::standard()), now()).await;

    assert_eq!(config.read(File::Lapses), "not a record");
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_is_left_for_the_next_pass() {
    let config = lapsed("lapse-unreached");

    let written = passed(
        &service(&config, "http://127.0.0.1:9", Limit::standard()),
        now(),
    )
    .await;

    assert!(written);
    assert!(config.read(File::Lapses).is_empty());
}
