//! An SFTP session whose connection goes away says so: `SftpManager` against an
//! in-process SSH server with an SFTP subsystem, which the test then hangs up.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use russh::keys::key::KeyPair;
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{StatusCode, Version};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use omnyssh_core::event::CoreEvent;
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::password::Prompter;
use omnyssh_core::ssh::sftp::SftpManager;

const PASSWORD: &str = "sftp-test";

/// Keeps trust-on-first-use off the real `~/.ssh`, and the local agent out.
fn isolate_home() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &home);
        std::env::remove_var("SSH_AUTH_SOCK");
    });
}

/// The server side of each connection, kept so the test can hang up.
type Sessions = Arc<Mutex<Vec<server::Handle>>>;

#[derive(Clone, Default)]
struct Server {
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

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

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.channels.lock().unwrap().insert(channel.id(), channel);
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = self.channels.lock().unwrap().remove(&channel_id);
        match channel {
            Some(channel) if name == "sftp" => {
                session.channel_success(channel_id);
                russh_sftp::server::run(channel.into_stream(), Sftp).await;
            }
            _ => session.channel_failure(channel_id),
        }
        Ok(())
    }
}

/// Answers the handshake; nothing else is needed here.
struct Sftp;

impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }
}

async fn serve() -> (SocketAddr, Sessions) {
    let config = Arc::new(server::Config {
        keys: vec![KeyPair::generate_ed25519()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let sessions = Sessions::default();
    let kept = Arc::clone(&sessions);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let config = Arc::clone(&config);
            let kept = Arc::clone(&kept);
            tokio::spawn(async move {
                if let Ok(running) = server::run_stream(config, socket, Server::default()).await {
                    kept.lock().unwrap().push(running.handle());
                    let _ = running.await;
                }
            });
        }
    });
    (addr, sessions)
}

async fn connect(addr: SocketAddr) -> (SftpManager, mpsc::Receiver<CoreEvent>) {
    isolate_home();
    let host = Host {
        name: "box".to_string(),
        hostname: addr.ip().to_string(),
        port: addr.port(),
        user: "me".to_string(),
        password: Some(PASSWORD.to_string()),
        ..Host::default()
    };
    let (tx, mut events) = mpsc::channel::<CoreEvent>(64);
    let (prompts, _) = mpsc::channel::<CoreEvent>(1);
    let manager = SftpManager::connect(&host, tx, Prompter::new(prompts, "box"))
        .await
        .expect("connect");
    match next(&mut events).await {
        Some(CoreEvent::SftpConnected { .. }) => {}
        other => panic!("expected SftpConnected, got {other:?}"),
    }
    (manager, events)
}

/// The next event, or `None` once the session has ended.
async fn next(events: &mut mpsc::Receiver<CoreEvent>) -> Option<CoreEvent> {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("no event within 10 s")
}

#[tokio::test]
async fn a_dropped_connection_ends_the_session_with_sftp_lost() {
    let (addr, sessions) = serve().await;
    let (_manager, mut events) = connect(addr).await;

    let server = sessions.lock().unwrap().pop().expect("a server session");
    server
        .disconnect(
            russh::Disconnect::ByApplication,
            String::new(),
            String::new(),
        )
        .await
        .expect("hang up");

    match next(&mut events).await {
        Some(CoreEvent::SftpLost { reason }) => assert!(reason.contains("box"), "{reason}"),
        other => panic!("expected SftpLost, got {other:?}"),
    }
    // The task has ended: nothing follows.
    assert!(next(&mut events).await.is_none());
}

#[tokio::test]
async fn closing_the_session_is_not_a_loss() {
    let (addr, _sessions) = serve().await;
    let (manager, mut events) = connect(addr).await;

    manager.disconnect();

    assert!(next(&mut events).await.is_none());
}
