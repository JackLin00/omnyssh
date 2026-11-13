//! Logging in with a key a key generator makes: `SshSession::connect` against an
//! in-process SSH server that takes one public key, or a password. A stand-in
//! frontend answers the key requests by the device's MAC.

use std::net::SocketAddr;
use std::sync::{Arc, Once, OnceLock};
use std::time::Duration;

use russh::keys::key::{KeyPair, PublicKey};
use russh::server::{self, Auth};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use omnyssh_core::config::key_generators::{save_key_generators, KeyGenerator, KeyGenerators};
use omnyssh_core::event::CoreEvent;
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::key_gen;
use omnyssh_core::ssh::session::SshSession;

const PASSWORD: &str = "keygen-test";

/// The key the server takes, as the generator returns it for MAC `good`.
fn device_key() -> &'static KeyPair {
    static KEY: OnceLock<KeyPair> = OnceLock::new();
    KEY.get_or_init(KeyPair::generate_ed25519)
}

fn pem(key: &KeyPair) -> String {
    let mut text = Vec::new();
    russh::keys::encode_pkcs8_pem(key, &mut text).unwrap();
    String::from_utf8(text).unwrap()
}

/// A temporary home with generators switched on and a frontend serving them:
/// MAC `good` gets the server's key, `wrong` another key, `broken` an error.
fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("SSH_AUTH_SOCK");
        save_key_generators(&KeyGenerators {
            enabled: true,
            generators: vec![KeyGenerator {
                name: "vendor-a".to_string(),
                code: "async function generateKey() {}".to_string(),
                credential: r#"{"master_key":"k"}"#.to_string(),
            }],
        })
        .expect("save generators");

        let (tx, mut rx) = mpsc::channel::<CoreEvent>(16);
        key_gen::serve(tx);
        let good = pem(device_key());
        let wrong = pem(&KeyPair::generate_ed25519());
        // A thread of its own: each test has its own runtime.
        std::thread::spawn(move || {
            while let Some(event) = rx.blocking_recv() {
                if let CoreEvent::KeyRequired {
                    request_id, mac, ..
                } = event
                {
                    let reply = match mac.as_str() {
                        "good" => Ok(good.clone()),
                        "wrong" => Ok(wrong.clone()),
                        _ => Err("TypeError: master_key is undefined".to_string()),
                    };
                    let _ = key_gen::answer(request_id, reply);
                }
            }
        });
    });
}

#[derive(Clone)]
struct Server;

#[async_trait::async_trait]
impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == PASSWORD {
            Auth::Accept
        } else {
            Auth::Reject {
                proceed_with_methods: None,
            }
        })
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(if *key == device_key().clone_public_key().unwrap() {
            Auth::Accept
        } else {
            Auth::Reject {
                proceed_with_methods: None,
            }
        })
    }
}

async fn serve() -> SocketAddr {
    let config = Arc::new(server::Config {
        keys: vec![KeyPair::generate_ed25519()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = Arc::clone(&config);
            tokio::spawn(async move {
                if let Ok(running) = server::run_stream(config, socket, Server).await {
                    let _ = running.await;
                }
            });
        }
    });
    addr
}

fn host(addr: SocketAddr, mac: &str, password: Option<&str>) -> Host {
    Host {
        name: format!("cam-{mac}"),
        hostname: addr.ip().to_string(),
        port: addr.port(),
        user: "root".to_string(),
        password: password.map(str::to_string),
        key_generator: Some("vendor-a".to_string()),
        device_mac: Some(mac.to_string()),
        ..Host::default()
    }
}

#[tokio::test]
async fn the_generated_key_logs_in() {
    setup();
    let addr = serve().await;
    let session = SshSession::connect(&host(addr, "good", None)).await;
    assert!(session.is_ok(), "{:#}", session.err().unwrap());
}

#[tokio::test]
async fn a_key_the_server_turns_down_falls_back_to_the_password() {
    setup();
    let addr = serve().await;
    let session = SshSession::connect(&host(addr, "wrong", Some(PASSWORD))).await;
    assert!(session.is_ok(), "{:#}", session.err().unwrap());
}

#[tokio::test]
async fn a_generator_that_fails_falls_back_to_the_password() {
    setup();
    let addr = serve().await;
    let session = SshSession::connect(&host(addr, "broken", Some(PASSWORD))).await;
    assert!(session.is_ok(), "{:#}", session.err().unwrap());
}

#[tokio::test]
async fn when_nothing_gets_in_the_error_says_what_the_generator_did() {
    setup();
    let addr = serve().await;
    let err = SshSession::connect(&host(addr, "wrong", Some("not-it")))
        .await
        .err()
        .expect("nothing should get in");
    let message = format!("{err:#}");
    assert!(
        message.contains("key generator 'vendor-a': the server did not accept its key"),
        "{message}"
    );
}

#[tokio::test]
async fn a_host_without_a_generator_does_not_use_one() {
    setup();
    let addr = serve().await;
    // The same device, but no generator named: its key would have got in.
    let mut plain = host(addr, "good", None);
    plain.key_generator = None;
    assert!(SshSession::connect(&plain).await.is_err());
}
