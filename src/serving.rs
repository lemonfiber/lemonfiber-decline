//! What the service answers, and the health check its image runs.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Path as Route, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lemonfiber_sidecar::decline::{Claimed, File, Key, Lapses, Refusals, Table};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::declining::{decline, standing, Standing};
use crate::jellyfin::Jellyfin;
use crate::lapsing::Taking;
use crate::limit::{Limit, Limiter};
use crate::page::{page, Said};
use crate::settings::Settings;

/// How long the media server has to answer one call. A refusal and a pass over lapsed
/// invitations share one lock, so a call left hanging would hold up every refusal
/// behind it.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);

/// What every request is answered with: the settings, the rate limit, and the one
/// lock that keeps two refusals, or a refusal and a pass over lapsed invitations, from
/// writing at once.
pub(crate) struct Service {
    settings: Settings,
    limiter: Mutex<Limiter>,
    recording: Mutex<()>,
}

impl Service {
    /// A service over `settings`, limited by `limit`.
    pub(crate) fn new(settings: Settings, limit: Limit) -> Arc<Self> {
        Arc::new(Self {
            settings,
            limiter: Mutex::new(Limiter::new(limit)),
            recording: Mutex::new(()),
        })
    }
}

/// The routes the service answers: its health, the page, and the refusal.
pub(crate) fn routes(service: Arc<Service>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/decline/{token}", get(shown).post(declined))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .with_state(service)
}

/// Serve [`routes`] on `at` until `stopped` resolves.
pub(crate) async fn serve(
    at: SocketAddr,
    service: Arc<Service>,
    stopped: impl std::future::Future<Output = ()> + Send + 'static,
) -> ExitCode {
    let listener = match TcpListener::bind(at).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("decline: could not listen on {at}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let app = routes(service).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(stopped)
        .await
        .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
}

/// Whether it is running, and which key it holds: the key's fingerprint, or nothing
/// where the core has not written one. The key is not used to answer this.
async fn health(State(service): State<Arc<Service>>) -> Json<serde_json::Value> {
    let key = read(&service.settings.config, File::Key)
        .await
        .and_then(|text| Key::read(&text).ok())
        .map(|key| key.fingerprint());
    Json(serde_json::json!({ "key": key }))
}

/// The page for the invitation `token` declines.
async fn shown(State(service): State<Arc<Service>>, Route(token): Route<String>) -> Response {
    let (table, refusals) = files(&service.settings.config).await;
    let standing = standing(&token, now(), &table, &refusals);
    answer(StatusCode::OK, &Said::Standing(&standing), &token)
}

/// Decline the invitation `token` names, and say what came of it.
async fn declined(
    State(service): State<Arc<Service>>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Route(token): Route<String>,
) -> Response {
    let at = now();
    if !service.limiter.lock().await.admits(from.ip(), at) {
        return answer(StatusCode::TOO_MANY_REQUESTS, &Said::Limited, &token);
    }

    let _recording = service.recording.lock().await;
    let config = &service.settings.config;
    let (table, refusals) = files(config).await;
    let Some(server) = media_server(&service.settings).await else {
        return answer(StatusCode::OK, &Said::Standing(&Standing::Refused), &token);
    };

    let (standing, after) = decline(&token, at, &table, refusals, &server).await;
    if standing == Standing::Declined && !recorded(config, File::Refusals, &after.written()).await {
        eprintln!("decline: the account was disabled and the refusal could not be recorded");
    }
    answer(StatusCode::OK, &Said::Standing(&standing), &token)
}

/// A page, with the headers that keep it from being framed, cached or used to load
/// anything.
fn answer(status: StatusCode, said: &Said<'_>, token: &str) -> Response {
    let mut response = (status, page(said, token)).into_response();
    let headers = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
             frame-ancestors 'none'; base-uri 'none'",
        ),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// The core's table and the refusals recorded so far. A table not yet written holds
/// no invitation, and a record not yet written holds no refusal; one that cannot be
/// read is treated the same, so nothing is declined on the strength of it.
async fn files(config: &Path) -> (Table, Refusals) {
    let table = read(config, File::Table)
        .await
        .and_then(|text| Table::read(&text).ok())
        .unwrap_or_else(|| Table::of(Claimed::Unknown, Vec::new()));
    let refusals = read(config, File::Refusals)
        .await
        .and_then(|text| Refusals::read(&text).ok())
        .unwrap_or_default();
    (table, refusals)
}

/// The text of `file` in `config`, where it can be read.
async fn read(config: &Path, file: File) -> Option<String> {
    tokio::fs::read_to_string(config.join(file.name()))
        .await
        .ok()
}

/// Write `text` to the record `file`, whole, by renaming a written copy over it, so the
/// core never reads half of one.
async fn recorded(config: &Path, file: File, text: &str) -> bool {
    let record = config.join(file.name());
    let written = config.join(format!("{}.writing", file.name()));
    tokio::fs::write(&written, text).await.is_ok()
        && tokio::fs::rename(&written, &record).await.is_ok()
}

/// How often the service looks for invitations whose window has closed.
const PASS: Duration = Duration::from_mins(1);

/// Take back the invitations whose window has closed, once every [`PASS`], for as long
/// as the service runs.
pub(crate) async fn lapsing(service: Arc<Service>) {
    let mut every = tokio::time::interval(PASS);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        if !passed(&service, now()).await {
            eprintln!("decline: invitations were taken back and that could not be recorded");
        }
    }
}

/// One pass over the invitations whose window had closed by `at`, and whether what it
/// did was recorded.
///
/// The key is read only where something is due, so a pass with nothing to take back
/// reaches nothing. Where the record of lapses cannot be read, the pass switches off
/// every account it would take back, removes none, and keeps the record as it is:
/// writing it afresh would lose what it held, and waiting for it would leave every lapsed
/// invitation claimable.
pub(crate) async fn passed(service: &Service, at: u64) -> bool {
    let _recording = service.recording.lock().await;
    let config = &service.settings.config;
    let (table, refusals) = files(config).await;
    let (lapses, taking) = match read(config, File::Lapses).await {
        Some(text) => match Lapses::read(&text) {
            Ok(lapses) => (lapses, Taking::Whole),
            Err(unreadable) => {
                eprintln!(
                    "decline: lapsed invitations are switched off and none removed, because {unreadable}"
                );
                (Lapses::default(), Taking::SwitchingOff)
            }
        },
        None => (Lapses::default(), Taking::Whole),
    };
    let due = table.invitations.iter().any(|invitation| {
        !invitation.open_at(at)
            && refusals.of(&invitation.token).is_none()
            && lapses.of(&invitation.token).is_none()
    });
    if !due {
        return true;
    }
    let Some(server) = media_server(&service.settings).await else {
        eprintln!(
            "decline: no invitation was taken back, because no key or no certificate pin is held for the media server"
        );
        return true;
    };
    let after = crate::lapsing::swept(
        at,
        &table,
        &refusals,
        lapses.clone(),
        &server,
        || table_in(config),
        taking,
    )
    .await;
    taking == Taking::SwitchingOff
        || after == lapses
        || recorded(config, File::Lapses, &after.written()).await
}

/// The media server as `settings` name it, reached with the key and over TLS pinned to the
/// certificates held, or nothing where either is not held: the key is never sent where
/// the certificate cannot be pinned.
async fn media_server(settings: &Settings) -> Option<Jellyfin> {
    let key = read(&settings.config, File::Key)
        .await
        .and_then(|text| Key::read(&text).ok())?;
    let pins = read(&settings.config, File::Pins)
        .await
        .and_then(|text| lemonfiber_sidecar::Pins::read(&text).ok())?;
    Some(Jellyfin {
        base: settings.jellyfin.clone(),
        key,
        client: lemonfiber_pinned::client(pins.listed(), Some(ANSWER_WITHIN))?,
    })
}

/// The core's table as `config` holds it now, where it can be read.
async fn table_in(config: &Path) -> Option<Table> {
    read(config, File::Table)
        .await
        .and_then(|text| Table::read(&text).ok())
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Whether the service on this container's `port` answers its health.
///
/// The image is distroless, with no shell and no HTTP client, so its health check
/// is this binary asking itself over loopback.
pub(crate) async fn healthy(port: u16) -> ExitCode {
    if answers_ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Whether `at` answers `GET /health` with `200`.
async fn answers_ok(at: SocketAddr) -> bool {
    health_answer(at)
        .await
        .is_some_and(|answer| answer.starts_with(b"HTTP/1.1 200"))
}

/// What `at` answers to `GET /health`, where it answers at all.
async fn health_answer(at: SocketAddr) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect(at).await.ok()?;
    let asked = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let written = stream.write_all(asked).await;
    let mut answer = Vec::new();
    let read = stream.read_to_end(&mut answer).await;
    (written.is_ok() && read.is_ok()).then_some(answer)
}

#[cfg(test)]
mod tests;
