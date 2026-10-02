//! What the service answers, and the health check its image runs.

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The routes this build answers: its health, and nothing else.
pub(crate) fn routes() -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .fallback(|| async { StatusCode::NOT_FOUND })
}

/// Serve [`routes`] on `at` until the container is told to stop.
pub(crate) async fn serve(at: SocketAddr) -> ExitCode {
    let listener = match TcpListener::bind(at).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("decline: could not listen on {at}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let stopped = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match axum::serve(listener, routes())
        .with_graceful_shutdown(stopped)
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("decline: stopped serving: {error}");
            ExitCode::FAILURE
        }
    }
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
    let Ok(mut stream) = TcpStream::connect(at).await else {
        return false;
    };
    let asked = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(asked).await.is_err() {
        return false;
    }
    let mut answer = Vec::new();
    if stream.read_to_end(&mut answer).await.is_err() {
        return false;
    }
    answer.starts_with(b"HTTP/1.1 200")
}

#[cfg(test)]
mod tests;
