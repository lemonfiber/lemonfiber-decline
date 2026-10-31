use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use crate::formats::decline::{File, Table, TokenHash};
use lemonfiber_adapter_kit::web::Web;
use lemonfiber_contract::capabilities::invite::decline;
use lemonfiber_contract::Contracted;
use lemonfiber_ports::service::{Claimed, Declinable, Declining, Invitations, KeyHeld};

use super::{about, kit, serve, UPSTREAM};

struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("speaking-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plugin_key() -> String {
    ('a'..='p').chain('A'..='P').collect()
}

fn media_key() -> String {
    ('g'..='z').collect()
}

async fn served(config: &Config) -> Option<SocketAddr> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .ok()?;
    let at = listener.local_addr().ok()?;
    let kit = kit(plugin_key(), &config.0);
    tokio::spawn(lemonfiber_adapter_kit::serving(
        kit,
        listener,
        std::future::pending(),
    ));
    Some(at)
}

fn asked(at: SocketAddr, key: &str) -> Option<decline::Adapter> {
    let web = Web::new().ok()?;
    Some(decline::Adapter(Contracted::new(
        Arc::new(web),
        format!("http://{at}"),
        "decline",
        key,
    )))
}

#[test]
fn it_says_it_speaks_invite_decline_for_itself() {
    let about = about();

    assert_eq!(about.speaks, vec!["invite.decline@1".to_owned()]);
    assert_eq!(about.upstream, UPSTREAM);
}

#[tokio::test]
async fn the_core_holds_it_to_a_key_and_applies_the_invitations_over_the_contract() {
    let config = Config::new("contract");
    let core = served(&config)
        .await
        .and_then(|at| asked(at, &plugin_key()));
    assert!(core.is_some());
    let Some(core) = core else { return };

    let pins = vec!["ef".repeat(32)];
    assert!(core.hold(&media_key(), &pins).await.is_ok());
    assert_eq!(
        core.held().await.ok().flatten(),
        Some(KeyHeld {
            key: TokenHash::of(&media_key()).as_str().to_owned(),
            pins,
        })
    );

    let invitations = Invitations {
        claimed: Claimed::OwnWrites,
        invitations: vec![Declinable {
            token: TokenHash::of("token").as_str().to_owned(),
            account: "4f1c".to_owned(),
            name: "Ana".to_owned(),
            issued: 1_000,
            lapses: 2_000,
        }],
    };
    assert!(core.apply_invitations(&invitations).await.is_ok());
    let table = Table::read(&config.read(File::Table)).ok();
    assert!(table
        .and_then(|table| table.find(&TokenHash::of("token")).cloned())
        .is_some());
}

#[tokio::test]
async fn a_caller_without_the_plugins_key_is_refused_and_changes_nothing() {
    let config = Config::new("unkeyed");
    let stranger = served(&config)
        .await
        .and_then(|at| asked(at, "not-the-key"));
    assert!(stranger.is_some());
    let Some(stranger) = stranger else { return };

    assert!(stranger
        .hold(&media_key(), &["ef".repeat(32)])
        .await
        .is_err());
    assert!(stranger.held().await.is_err());
    assert_eq!(config.read(File::Key), "");
}

#[tokio::test]
async fn with_no_key_written_nothing_answers_on_the_contract_port() {
    let config = Config::new("keyless");
    let free =
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|probe| probe.local_addr());
    assert!(free.is_ok());
    let Ok(at) = free else { return };

    let waited =
        tokio::time::timeout(std::time::Duration::from_millis(200), serve(at, &config.0)).await;

    assert!(waited.is_err());
    assert!(tokio::net::TcpStream::connect(at).await.is_err());
}

fn keyed(config: &Config) {
    let _ = std::fs::write(
        config.0.join(lemonfiber_contract::adapter::KEY_FILE),
        format!("{}\n", plugin_key()),
    );
}

#[tokio::test]
async fn with_its_key_written_it_answers_the_core_on_the_contract_port() {
    let config = Config::new("serving");
    keyed(&config);
    let free =
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|probe| probe.local_addr());
    assert!(free.is_ok());
    let Ok(at) = free else { return };

    let path = config.0.clone();
    let running = tokio::spawn(async move { serve(at, &path).await });
    let core = asked(at, &plugin_key());
    let mut held = None;
    for _ in 0..250 {
        if let Some(core) = &core {
            if let Ok(answer) = core.held().await {
                held = Some(answer);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    running.abort();

    assert_eq!(held, Some(None));
}

#[tokio::test]
async fn a_contract_port_already_taken_is_a_failure_to_serve() {
    let config = Config::new("taken");
    keyed(&config);
    let taken = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await;
    assert!(taken.is_ok());
    let Ok(taken) = taken else { return };
    let Ok(at) = taken.local_addr() else { return };

    assert!(serve(at, &config.0).await.is_err());
}
