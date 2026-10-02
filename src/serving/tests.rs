use std::net::{Ipv4Addr, SocketAddr};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::{answers_ok, routes};

async fn status_of(method: &str, path: &str) -> StatusCode {
    let asked = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap_or_default();
    routes()
        .oneshot(asked)
        .await
        .map_or(StatusCode::INTERNAL_SERVER_ERROR, |answer| answer.status())
}

#[tokio::test]
async fn health_answers_ok() {
    assert_eq!(status_of("GET", "/health").await, StatusCode::OK);
}

#[tokio::test]
async fn anything_else_is_not_found() {
    assert_eq!(status_of("GET", "/").await, StatusCode::NOT_FOUND);
    assert_eq!(
        status_of("GET", "/decline/anything").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn health_is_only_asked_for() {
    assert_eq!(
        status_of("POST", "/health").await,
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[tokio::test]
async fn the_health_check_reads_a_serving_service_as_healthy() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    tokio::spawn(async move { axum::serve(listener, routes()).await });

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
