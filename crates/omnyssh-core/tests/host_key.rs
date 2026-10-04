//! A host key that changed, end to end: a terminal that asks about it, against
//! an in-process SSH server whose key each test picks per connection.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use russh::keys::key::{KeyPair, PublicKey};
use russh::keys::PublicKeyBase64;
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use omnyssh_core::event::CoreEvent;
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::host_key::{self, Decision};
use omnyssh_core::ssh::pty::PtyManager;
use omnyssh_core::ssh::session::SshSession;

const PASSWORD: &str = "host-key-test";

/// One test at a time: they share `~/.ssh/known_hosts`, and one rewriting it
/// must not drop a line another has just added.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Keeps the tests off the real `~/.ssh`, and the local agent's keys out.
fn isolate_home() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var("SSH_AUTH_SOCK");
    });
}

fn known_hosts() -> PathBuf {
    std::env::home_dir()
        .expect("home")
        .join(".ssh")
        .join("known_hosts")
}

fn public(pair: &KeyPair) -> PublicKey {
    pair.clone_public_key().expect("public key")
}

fn fingerprint(pair: &KeyPair) -> String {
    format!("SHA256:{}", public(pair).fingerprint())
}

/// Saves `pair` as the key `addr` had before.
fn pin(addr: SocketAddr, pair: &KeyPair) {
    russh::keys::known_hosts::learn_known_hosts_path(
        &addr.ip().to_string(),
        addr.port(),
        &public(pair),
        known_hosts(),
    )
    .expect("pin the old key");
}

fn pinned(addr: SocketAddr, pair: &KeyPair) -> bool {
    let line = format!(
        "[{}]:{} ssh-ed25519 {}",
        addr.ip(),
        addr.port(),
        public(pair).public_key_base64()
    );
    std::fs::read_to_string(known_hosts())
        .unwrap_or_default()
        .lines()
        .any(|l| l == line)
}

/// Takes the password and opens a shell that prints `logged-in`.
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

    async fn auth_publickey(&mut self, _user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        Ok(Auth::Reject {
            proceed_with_methods: None,
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, CryptoVec::from_slice(b"logged-in\r\n"));
        Ok(())
    }
}

/// Serves on a loopback port, showing `keys[n]` to the n-th connection and the
/// last one from then on. Also returns how many connections it took.
async fn serve(keys: Vec<KeyPair>) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let connections = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&connections);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let n = count.fetch_add(1, Ordering::SeqCst);
            let config = Arc::new(server::Config {
                keys: vec![keys[n.min(keys.len() - 1)].clone()],
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Default::default()
            });
            let _ = server::run_stream(config, socket, Server).await;
        }
    });
    (addr, connections)
}

/// What the frontend saw: each question (saved fingerprints, offered one), and
/// each error.
#[derive(Default)]
struct Seen {
    asked: Vec<(Vec<String>, String)>,
    errors: Vec<String>,
}

/// Plays the frontend: answers each host-key question with the next of
/// `decisions` (then Cancel), and notes what it saw.
fn frontend(mut rx: mpsc::Receiver<CoreEvent>, decisions: Vec<Decision>) -> Arc<Mutex<Seen>> {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let noted = Arc::clone(&seen);
    tokio::spawn(async move {
        let mut decisions = decisions.into_iter();
        while let Some(event) = rx.recv().await {
            match event {
                CoreEvent::HostKeyChanged {
                    request_id,
                    saved,
                    offered,
                    ..
                } => {
                    noted.lock().unwrap().asked.push((
                        saved.into_iter().map(|k| k.fingerprint).collect(),
                        offered.fingerprint,
                    ));
                    let decision = decisions.next().unwrap_or(Decision::Cancel);
                    host_key::answer(request_id, decision).expect("the connection waits");
                }
                CoreEvent::Error(message) => noted.lock().unwrap().errors.push(message),
                _ => {}
            }
        }
    });
    seen
}

/// Opens a terminal that asks about host keys, answered with `decisions`.
fn open(
    name: &str,
    addr: SocketAddr,
    decisions: Vec<Decision>,
) -> (PtyManager, u64, Arc<Mutex<Seen>>) {
    open_with(name, addr, Some(PASSWORD), decisions)
}

/// [`open`], with `password` saved for the host (or none).
fn open_with(
    name: &str,
    addr: SocketAddr,
    password: Option<&str>,
    decisions: Vec<Decision>,
) -> (PtyManager, u64, Arc<Mutex<Seen>>) {
    let (tx, rx) = mpsc::channel(256);
    let seen = frontend(rx, decisions);
    let mut pty = PtyManager::new().asking_host_keys();
    let id = pty
        .open(&host(name, addr, password), 80, 24, tx)
        .expect("open");
    (pty, id, seen)
}

/// The host `name` at `addr`, logging in as `name`, with `password` saved for
/// it (or none).
fn host(name: &str, addr: SocketAddr, password: Option<&str>) -> Host {
    Host {
        name: name.to_string(),
        hostname: addr.ip().to_string(),
        port: addr.port(),
        user: name.to_string(),
        password: password.map(str::to_string),
        ..Host::default()
    }
}

/// Types `text` and Enter once the password prompt shows.
async fn type_password(pty: &mut PtyManager, id: u64, text: &str) {
    assert!(
        screen_contains(pty, id, "'s password: ").await,
        "no password prompt"
    );
    pty.write(id, format!("{text}\r").as_bytes()).expect("type");
}

async fn screen_contains(pty: &PtyManager, id: u64, text: &str) -> bool {
    for _ in 0..100 {
        if let Some(parser) = pty.parser_for(id) {
            if parser.lock().unwrap().screen().contents().contains(text) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn screen(pty: &PtyManager, id: u64) -> String {
    let parser = pty.parser_for(id).expect("tab");
    let contents = parser.lock().unwrap().screen().contents();
    contents
}

async fn first_error(seen: &Mutex<Seen>) -> String {
    for _ in 0..100 {
        if let Some(error) = seen.lock().unwrap().errors.first().cloned() {
            return error;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no error reported");
}

/// Update puts the new key in place of the old one. The saved password is not
/// sent on its own to the key just let in: the login asks for it. From then on
/// the key is trusted, and the next connections, background ones too, log in
/// with the saved password as before.
#[tokio::test]
async fn update_saves_the_new_key_and_connects() {
    isolate_home();
    let _one = SERIAL.lock().await;
    let (old, new) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
    let (addr, _) = serve(vec![new.clone()]).await;
    pin(addr, &old);

    let (mut pty, id, seen) = open("update", addr, vec![Decision::Update]);
    type_password(&mut pty, id, PASSWORD).await;
    assert!(screen_contains(&pty, id, "logged-in").await, "no shell");
    assert_eq!(
        seen.lock().unwrap().asked,
        [(vec![fingerprint(&old)], fingerprint(&new))]
    );
    assert!(pinned(addr, &new), "the new key is saved");
    assert!(!pinned(addr, &old), "the old key is gone");

    let (pty, id, seen) = open("update", addr, vec![]);
    assert!(screen_contains(&pty, id, "logged-in").await, "no shell");
    assert!(seen.lock().unwrap().asked.is_empty());
    assert!(!screen(&pty, id).contains("'s password: "), "asked again");
    SshSession::connect(&host("update", addr, Some(PASSWORD)))
        .await
        .expect("background work logs in with the saved password");
}

/// Once lets the new key in for that connection only: nothing is saved, the
/// saved password is asked for rather than sent, and the next connection asks
/// again. Cancelling it fails with the headline only.
#[tokio::test]
async fn once_saves_nothing_and_cancel_gives_up() {
    isolate_home();
    let _one = SERIAL.lock().await;
    let (old, new) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
    let (addr, _) = serve(vec![new.clone()]).await;
    pin(addr, &old);

    let (mut pty, id, _seen) = open("once", addr, vec![Decision::Once]);
    type_password(&mut pty, id, PASSWORD).await;
    assert!(screen_contains(&pty, id, "logged-in").await, "no shell");
    assert!(
        pinned(addr, &old) && !pinned(addr, &new),
        "nothing is saved"
    );

    let (_pty, _id, seen) = open("once", addr, vec![Decision::Cancel]);
    assert_eq!(
        first_error(&seen).await,
        format!(
            "Terminal: Host key of {} port {} has changed",
            addr.ip(),
            addr.port()
        )
    );
    assert_eq!(seen.lock().unwrap().asked.len(), 1, "asked again");
}

/// A different key on the connection after an answer is asked about again,
/// never let in on the strength of the first answer — and a server that keeps
/// changing its key is asked about twice at most, then turned down in short.
#[tokio::test]
async fn a_key_that_changes_again_is_asked_about_again_at_most_twice() {
    isolate_home();
    let _one = SERIAL.lock().await;
    let old = KeyPair::generate_ed25519();
    let keys: Vec<KeyPair> = (0..3).map(|_| KeyPair::generate_ed25519()).collect();
    let (addr, connections) = serve(keys.clone()).await;
    pin(addr, &old);

    let (_pty, _id, seen) = open("rotating", addr, vec![Decision::Once; 3]);
    assert_eq!(
        first_error(&seen).await,
        format!(
            "Terminal: Host key of {} port {} keeps changing; not connecting",
            addr.ip(),
            addr.port()
        )
    );
    assert_eq!(
        seen.lock().unwrap().asked,
        [
            (vec![fingerprint(&old)], fingerprint(&keys[0])),
            (vec![fingerprint(&old)], fingerprint(&keys[1])),
        ]
    );
    assert_eq!(connections.load(Ordering::SeqCst), 3);
    assert!(pinned(addr, &old), "nothing is saved");
}

/// A password typed for a login is remembered with the host key it went to,
/// and background work (`SshSession::connect`) logs in with it for as long as
/// the server shows that key.
#[tokio::test]
async fn a_remembered_password_serves_later_connections_to_the_same_key() {
    isolate_home();
    let _one = SERIAL.lock().await;
    let key = KeyPair::generate_ed25519();
    let (addr, _) = serve(vec![key.clone()]).await;
    pin(addr, &key);

    let (mut pty, id, _seen) = open_with("same-key", addr, None, vec![]);
    type_password(&mut pty, id, PASSWORD).await;
    assert!(screen_contains(&pty, id, "logged-in").await, "no shell");

    SshSession::connect(&host("same-key", addr, None))
        .await
        .expect("the remembered password logs in again");
}

/// After the user lets a changed key in, a password typed for the login earlier
/// in the session is not sent on its own: if the change is an interception, it
/// would go to whoever intercepts. The login asks for it again instead — and,
/// once the key is saved, background work does not send it either.
#[tokio::test]
async fn a_remembered_password_is_not_sent_after_a_changed_key_is_let_in() {
    isolate_home();
    let _one = SERIAL.lock().await;
    for decision in [Decision::Update, Decision::Once] {
        let (old, new) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
        // The first connection shows the pinned key, every later one the new key.
        let (addr, _) = serve(vec![old.clone(), new.clone()]).await;
        pin(addr, &old);
        let name = format!("remembered-{decision:?}").to_lowercase();

        // Typed once at the prompt: remembered for the login.
        let (mut pty, id, _seen) = open_with(&name, addr, None, vec![]);
        type_password(&mut pty, id, PASSWORD).await;
        assert!(screen_contains(&pty, id, "logged-in").await, "no shell");

        // The key changed and the user let it in: the password is asked again.
        // The user gives up at the prompt (Ctrl+C).
        let (mut pty, id, seen) = open_with(&name, addr, None, vec![decision]);
        assert!(
            screen_contains(&pty, id, "'s password: ").await,
            "{decision:?}: the remembered password was sent to the changed key"
        );
        assert_eq!(seen.lock().unwrap().asked.len(), 1);
        pty.write(id, b"\x03").expect("type");
        let error = first_error(&seen).await;
        assert!(error.contains("cancelled"), "{error}");

        if decision == Decision::Update {
            // The new key is saved, but the password was remembered with the
            // old one: background work does not send it.
            assert!(pinned(addr, &new));
            let e = SshSession::connect(&host(&name, addr, None))
                .await
                .err()
                .expect("no password to log in with");
            assert!(format!("{e:#}").contains("no password is saved"), "{e:#}");
        }
    }
}
