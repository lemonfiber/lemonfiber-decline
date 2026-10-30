use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use lemonfiber_sidecar::decline::{
    Claimed, File, Invitation, Key, Lapses, Outcome, Refusals, Table, TokenHash,
};
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::{answers_ok, lapsing, now, passed, routes, Service};
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

    fn mode(&self, file: File) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(self.0.join(file.name()))
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or_default()
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    fn with_invitation(self) -> Self {
        let table = Table::of(
            Claimed::HasPassword,
            vec![Invitation {
                token: TokenHash::of("token"),
                account: "known".to_owned(),
                name: "Ana".to_owned(),
                issued: now() - 60,
                lapses: now() + 3_600,
            }],
        );
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
            jellyfin: lemonfiber_pinned::Https::parse(jellyfin)
                .or_else(|| lemonfiber_pinned::Https::parse(NOWHERE))
                .unwrap_or_else(|| unreachable!("an HTTPS address")),
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

/// An HTTPS address nothing answers at.
const NOWHERE: &str = "https://127.0.0.1:9";

/// `app` served over TLS as the media server, its certificate pinned in `config`.
fn served(config: &Config, app: Router) -> String {
    let Some(served) = lemonfiber_pinned::fake::tls(app) else {
        return String::new();
    };
    let _ = std::fs::write(
        config
            .0
            .join(lemonfiber_sidecar::decline::File::Pins.name()),
        format!("{}\n", served.pin),
    );
    served.base.to_string()
}

/// A media server that holds the account `known`, unclaimed, and takes its policy.
fn jellyfin(config: &Config) -> String {
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
    served(config, app)
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
    let server = jellyfin(&config);

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
    assert_eq!(config.mode(File::Refusals), 0o600);

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
async fn without_a_pinned_certificate_the_key_is_never_sent() {
    let config = Config::new("unpinned").with_invitation();
    config.write(File::Key, "the-key\n");
    let (server, writes) = jellyfin_remembering_writes(&config);

    for pins in [None, Some("not a pin\n")] {
        match pins {
            None => {
                let _ = std::fs::remove_file(
                    config
                        .0
                        .join(lemonfiber_sidecar::decline::File::Pins.name()),
                );
            }
            Some(text) => {
                let _ = std::fs::write(
                    config
                        .0
                        .join(lemonfiber_sidecar::decline::File::Pins.name()),
                    text,
                );
            }
        }
        let (_, body, _) = asked(
            service(&config, &server, Limit::standard()),
            "POST",
            "/decline/token",
        )
        .await;
        assert!(body.contains("This invitation cannot be declined here."));
    }
    assert!(config.read(File::Refusals).is_empty());
    assert!(writes.lock().is_ok_and(|one| one.is_empty()));
}

#[tokio::test]
async fn a_media_server_presenting_another_certificate_is_never_sent_the_key() {
    let config = Config::new("mispinned").with_invitation();
    config.write(File::Key, "the-key\n");
    let (server, writes) = jellyfin_remembering_writes(&config);
    let _ = std::fs::write(
        config
            .0
            .join(lemonfiber_sidecar::decline::File::Pins.name()),
        format!("{}\n", lemonfiber_pinned::sha256(b"another certificate")),
    );

    let (_, body, _) = asked(
        service(&config, &server, Limit::standard()),
        "POST",
        "/decline/token",
    )
    .await;

    assert!(body.contains("The media server did not answer, so nothing was declined."));
    assert!(config.read(File::Refusals).is_empty());
    assert!(writes.lock().is_ok_and(|one| one.is_empty()));
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
async fn the_household_port_answers_no_contract_route() {
    let config = Config::new("no-contract");
    for (method, path) in [
        ("GET", "/lemonfiber/adapter/v1/about"),
        ("POST", "/lemonfiber/invite.decline/v1/hold"),
        ("POST", "/lemonfiber/invite.decline/v1/held"),
        ("POST", "/lemonfiber/invite.decline/v1/apply_invitations"),
    ] {
        let (status, _, _) = asked(service(&config, "", Limit::standard()), method, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    assert_eq!(config.read(File::Key), "");
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
    let server = jellyfin(&config);
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
fn jellyfin_holding_an_offer(config: &Config) -> String {
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
    served(config, app)
}

/// A configuration directory holding a key and one invitation whose window closed a
/// minute ago.
fn lapsed(name: &str) -> Config {
    let config = Config::new(name);
    let table = Table::of(
        Claimed::HasPassword,
        vec![Invitation {
            token: TokenHash::of("token"),
            account: "known".to_owned(),
            name: "Ana".to_owned(),
            issued: now() - 3_600,
            lapses: now() - 60,
        }],
    );
    config.write(File::Table, &table.written());
    config.write(File::Key, "the-key\n");
    config
}

#[tokio::test]
async fn a_pass_takes_back_a_lapsed_invitation_and_records_it_for_the_core() {
    let config = lapsed("lapse-pass");
    let server = jellyfin_holding_an_offer(&config);

    let written = passed(&service(&config, &server, Limit::standard()), now()).await;

    let lapses = Lapses::read(&config.read(File::Lapses));
    assert!(written);
    assert_eq!(config.mode(File::Lapses), 0o600);
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

    let written = passed(&service(&config, NOWHERE, Limit::standard()), now()).await;

    assert!(written);
    assert!(config.read(File::Lapses).is_empty());
}

/// A media server holding the account `known` as [`jellyfin_holding_an_offer`] does, which
/// remembers each write it is asked for.
fn jellyfin_remembering_writes(config: &Config) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (removed, disabled) = (Arc::clone(&writes), Arc::clone(&writes));
    let app = Router::new()
        .route(
            "/Users/{id}",
            get(|| async {
                Json(json!({"Id": "known", "HasPassword": false,
                    "Policy": {"IsAdministrator": false, "IsDisabled": false}}))
            })
            .merge(delete(move || async move {
                if let Ok(mut writes) = removed.lock() {
                    writes.push("remove".to_owned());
                }
                StatusCode::NO_CONTENT
            })),
        )
        .route(
            "/Users/{id}/Policy",
            post(move || async move {
                if let Ok(mut writes) = disabled.lock() {
                    writes.push("disable".to_owned());
                }
                StatusCode::NO_CONTENT
            }),
        )
        .route(
            "/System/ActivityLog/Entries",
            get(|| async { Json(json!({"Items": []})) }),
        );
    (served(config, app), writes)
}

#[tokio::test]
async fn a_record_of_lapses_that_cannot_be_read_is_kept_and_the_lapsed_switched_off() {
    let config = lapsed("lapse-garbled");
    config.write(File::Lapses, "not a record");
    let (server, writes) = jellyfin_remembering_writes(&config);

    let written = passed(&service(&config, &server, Limit::standard()), now()).await;

    assert!(written);
    assert_eq!(config.read(File::Lapses), "not a record");
    assert_eq!(
        writes.lock().map(|one| one.clone()).unwrap_or_default(),
        vec!["disable".to_owned()]
    );
}

#[tokio::test]
async fn a_server_that_cannot_be_reached_is_left_for_the_next_pass() {
    let config = lapsed("lapse-unreached");

    let written = passed(&service(&config, NOWHERE, Limit::standard()), now()).await;

    assert!(written);
    assert!(config.read(File::Lapses).is_empty());
}

#[tokio::test]
async fn a_pass_adds_to_the_lapses_already_recorded() {
    let config = lapsed("lapse-added");
    let earlier = Lapses::default().with(lemonfiber_sidecar::decline::Lapse {
        token: TokenHash::of("earlier"),
        account: "gone".to_owned(),
        name: "Bo".to_owned(),
        issued: 1_000,
        at: 2_000,
        outcome: Outcome::Removed,
    });
    config.write(File::Lapses, &earlier.written());
    let server = jellyfin_holding_an_offer(&config);

    let written = passed(&service(&config, &server, Limit::standard()), now()).await;

    let lapses = Lapses::read(&config.read(File::Lapses)).ok();
    assert!(written);
    assert!(lapses
        .as_ref()
        .is_some_and(|lapses| lapses.of(&TokenHash::of("earlier")).is_some()));
    assert_eq!(
        lapses.and_then(|lapses| lapses.of(&TokenHash::of("token")).map(|one| one.outcome)),
        Some(Outcome::Removed)
    );
}

#[tokio::test]
async fn without_its_key_a_pass_takes_nothing_back() {
    let config = lapsed("lapse-keyless");
    let _ = std::fs::remove_file(config.0.join(File::Key.name()));
    let server = jellyfin_holding_an_offer(&config);

    let written = passed(&service(&config, &server, Limit::standard()), now()).await;

    assert!(written);
    assert!(config.read(File::Lapses).is_empty());
}

/// Whether `done` holds within five seconds, asked every 20 milliseconds.
async fn within_seconds(done: impl Fn() -> bool) -> bool {
    for _ in 0..250 {
        if done() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    done()
}

#[tokio::test]
async fn the_service_takes_back_a_lapsed_invitation_as_soon_as_it_starts() {
    let config = lapsed("lapse-running");
    let server = jellyfin_holding_an_offer(&config);

    let running = tokio::spawn(lapsing(service(&config, &server, Limit::standard())));
    let recorded = within_seconds(|| !config.read(File::Lapses).is_empty()).await;
    running.abort();

    assert!(recorded);
}

#[tokio::test]
async fn a_lapse_that_cannot_be_recorded_leaves_the_service_running() {
    let config = lapsed("lapse-unrecorded");
    let _ = std::fs::create_dir(config.0.join(format!("{}.writing", File::Lapses.name())));
    let server = jellyfin_holding_an_offer(&config);

    let running = tokio::spawn(lapsing(service(&config, &server, Limit::standard())));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let still_running = !running.is_finished();
    running.abort();

    assert!(still_running);
    assert!(config.read(File::Lapses).is_empty());
}
