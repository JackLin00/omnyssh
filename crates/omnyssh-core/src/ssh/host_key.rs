//! A host key that no longer matches `known_hosts`.
//!
//! The connection is turned down first, never held open mid-handshake while the
//! user decides: a server gives a login two minutes (OpenSSH's `LoginGraceTime`).
//! The key the server showed stays in the backend; frontends only ever see its
//! fingerprint. A connection the user started asks ([`CoreEvent::HostKeyChanged`])
//! and, as answered, saves the new key and connects again, connects again letting
//! that one key in once, or gives up.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use russh::keys::key::PublicKey;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::event::{CoreEvent, KeyFingerprint};
use crate::ssh::known_hosts;

/// How a refused changed key is put in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    /// The whole story and the `ssh-keygen -R` remedy, for a connection the user
    /// watches but nobody can ask about (the TUI's terminal and files).
    Full,
    /// Background work (pollers, tunnels, snippets, key setup): the headline and
    /// where to go. No ':' — frontends cut messages there.
    Background,
    /// The user said no, or did not answer: the headline is all they need.
    Declined,
    /// Asked about as often as one connection asks, and the server shows yet
    /// another key: the user has seen the story, a short refusal will do.
    KeepsChanging,
}

/// A connection turned down because the server's host key no longer matches the
/// one saved for it.
#[derive(Debug, Clone)]
pub(crate) struct HostKeyChanged {
    /// The hostname and port the key is pinned under: the hop's own, even
    /// through a tunnel.
    pub(crate) host: String,
    pub(crate) port: u16,
    /// The `known_hosts` file whose key no longer matches.
    pub(crate) file: PathBuf,
    /// The keys of the offered type saved there for the host.
    pub(crate) saved: Vec<PublicKey>,
    /// The key the server showed. Never leaves the backend.
    pub(crate) offered: PublicKey,
    /// The host a bastion was on the way to, when the key was met on one.
    pub(crate) jump_for: Option<String>,
    /// Where in a jump chain the connection failed.
    context: Option<String>,
    shown: Shown,
}

impl HostKeyChanged {
    pub(crate) fn new(
        host: &str,
        port: u16,
        file: PathBuf,
        saved: Vec<PublicKey>,
        offered: PublicKey,
    ) -> Self {
        Self {
            host: host.to_string(),
            port,
            file,
            saved,
            offered,
            jump_for: None,
            context: None,
            shown: Shown::Full,
        }
    }

    /// Puts `context` (where in a jump chain it failed) in front of the message.
    pub(crate) fn within(mut self, context: String) -> Self {
        self.context = Some(match self.context.take() {
            Some(inner) => format!("{context}: {inner}"),
            None => context,
        });
        self
    }

    /// Marks the key as met on a bastion on the way to `target`.
    pub(crate) fn jump_host_for(mut self, target: &str) -> Self {
        self.jump_for = Some(target.to_string());
        self
    }

    pub(crate) fn shown(mut self, shown: Shown) -> Self {
        self.shown = shown;
        self
    }
}

impl fmt::Display for HostKeyChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(context) = &self.context {
            write!(f, "{context}: ")?;
        }
        let who = known_hosts::who(&self.host, self.port);
        match self.shown {
            Shown::Full => f.write_str(&known_hosts::changed_message(
                &self.host,
                self.port,
                &self.file,
                &fingerprint(&self.offered),
            )),
            Shown::Background => write!(
                f,
                "Host key of {who} has changed (open a terminal to review the new key)"
            ),
            Shown::Declined => write!(f, "Host key of {who} has changed"),
            Shown::KeepsChanging => write!(f, "Host key of {who} keeps changing; not connecting"),
        }
    }
}

impl std::error::Error for HostKeyChanged {}

/// `SHA256:…`, as `ssh-keygen -l` prints it.
pub(crate) fn fingerprint(key: &PublicKey) -> String {
    format!("SHA256:{}", key.fingerprint())
}

/// The changed host key a failed connection was turned down for, if that is why.
pub(crate) fn changed(e: &anyhow::Error) -> Option<&HostKeyChanged> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<HostKeyChanged>())
}

/// What the user chose for a changed host key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Put the new key in `~/.ssh/known_hosts` in place of the old one, and
    /// connect.
    Update,
    /// Connect letting the new key in this once; nothing is saved.
    Once,
    /// Give the connection up.
    Cancel,
}

/// How long a question waits, as long as a password prompt does: nothing else
/// would end it if the frontend lost it (a reloaded page).
const PROMPT_TIMEOUT: Duration = Duration::from_secs(300);

/// Questions waiting for an answer, by request id. Apart from the password
/// prompts' own: the two never answer each other.
#[derive(Default)]
struct State {
    pending: HashMap<u64, oneshot::Sender<Decision>>,
    last_request: u64,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// An answer that has no question to go to.
#[derive(Debug, Error)]
pub enum HostKeyError {
    /// The question was answered already, or its connection stopped waiting.
    #[error("no connection is waiting for this answer")]
    NotRequested,
}

/// Answers the question `request_id` of a [`CoreEvent::HostKeyChanged`].
///
/// # Errors
/// [`HostKeyError::NotRequested`] when no connection waits on that request.
pub fn answer(request_id: u64, decision: Decision) -> Result<(), HostKeyError> {
    let reply = state()
        .pending
        .remove(&request_id)
        .ok_or(HostKeyError::NotRequested)?;
    reply.send(decision).map_err(|_| HostKeyError::NotRequested)
}

/// Asks the user about a changed host key.
#[async_trait]
pub(crate) trait AskHostKey: Send + Sync {
    /// The user's choice; [`Decision::Cancel`] when nobody answers.
    async fn ask(&self, changed: &HostKeyChanged) -> Decision;
}

/// Asks through the frontends: sends [`CoreEvent::HostKeyChanged`] and waits for
/// [`answer`].
pub struct HostKeyPrompter {
    tx: mpsc::Sender<CoreEvent>,
    host_name: String,
}

impl HostKeyPrompter {
    /// A prompter for connections to `host_name`, reporting on `tx`.
    pub fn new(tx: mpsc::Sender<CoreEvent>, host_name: impl Into<String>) -> Self {
        Self {
            tx,
            host_name: host_name.into(),
        }
    }
}

#[async_trait]
impl AskHostKey for HostKeyPrompter {
    async fn ask(&self, changed: &HostKeyChanged) -> Decision {
        let (reply, answer) = oneshot::channel();
        let request_id = {
            let mut state = state();
            state.last_request += 1;
            let id = state.last_request;
            state.pending.insert(id, reply);
            id
        };
        let _pending = Pending(request_id);
        let asked = self
            .tx
            .send(CoreEvent::HostKeyChanged {
                request_id,
                host_name: self.host_name.clone(),
                host: changed.host.clone(),
                port: changed.port,
                jump_for: changed.jump_for.clone(),
                file: changed.file.display().to_string(),
                saved: changed.saved.iter().map(shown_key).collect(),
                offered: shown_key(&changed.offered),
            })
            .await;
        if asked.is_err() {
            return Decision::Cancel;
        }
        // Answering an expired question later fails with NotRequested, which
        // frontends report and close it on.
        match tokio::time::timeout(PROMPT_TIMEOUT, answer).await {
            Ok(Ok(decision)) => decision,
            _ => Decision::Cancel,
        }
    }
}

fn shown_key(key: &PublicKey) -> KeyFingerprint {
    KeyFingerprint {
        key_type: known_hosts::key_type(key).to_string(),
        fingerprint: fingerprint(key),
    }
}

/// A question waiting for its answer; unlisted once the connection stops
/// waiting, so a late answer is refused.
struct Pending(u64);

impl Drop for Pending {
    fn drop(&mut self) {
        state().pending.remove(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CoreEvent, KeyFingerprint};
    use russh::keys::key::KeyPair;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn key() -> PublicKey {
        KeyPair::generate_ed25519()
            .clone_public_key()
            .expect("public key")
    }

    fn changed_key(port: u16) -> HostKeyChanged {
        HostKeyChanged::new(
            "10.0.0.5",
            port,
            PathBuf::from("/home/me/.ssh/known_hosts"),
            vec![key()],
            key(),
        )
    }

    #[test]
    fn by_default_the_refusal_tells_the_whole_story() {
        let changed = changed_key(2222);
        let message = changed.to_string();
        // The TUI status bar keeps what comes before the first ':'.
        assert_eq!(
            message.split(':').next(),
            Some("Host key of 10.0.0.5 port 2222 has changed")
        );
        assert!(message.contains(&fingerprint(&changed.offered)));
        assert!(message.contains("ssh-keygen -R \"[10.0.0.5]:2222\""));
    }

    #[test]
    fn background_work_just_says_where_to_look() {
        assert_eq!(
            changed_key(22).shown(Shown::Background).to_string(),
            "Host key of 10.0.0.5 has changed (open a terminal to review the new key)"
        );
    }

    #[test]
    fn a_jump_chain_puts_where_it_failed_first() {
        let message = changed_key(22)
            .within(String::from("connecting via 'b' failed"))
            .within(String::from("ProxyJump via 'a' failed"))
            .shown(Shown::Background)
            .to_string();
        assert!(
            message.starts_with(
                "ProxyJump via 'a' failed: connecting via 'b' failed: Host key of 10.0.0.5 has changed"
            ),
            "{message}"
        );
    }

    async fn asked(rx: &mut mpsc::Receiver<CoreEvent>) -> CoreEvent {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(event @ CoreEvent::HostKeyChanged { .. })) => event,
            other => panic!("expected a host-key question, got {other:?}"),
        }
    }

    fn request_id(event: &CoreEvent) -> u64 {
        match event {
            CoreEvent::HostKeyChanged { request_id, .. } => *request_id,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn the_question_shows_both_keys_and_the_answer_reaches_the_connection() {
        let (tx, mut rx) = mpsc::channel(8);
        let changed = changed_key(2222).jump_host_for("db");
        let (saved, offered) = (changed.saved[0].clone(), changed.offered.clone());
        let asking =
            tokio::spawn(async move { HostKeyPrompter::new(tx, "db").ask(&changed).await });

        let event = asked(&mut rx).await;
        match &event {
            CoreEvent::HostKeyChanged {
                host_name,
                host,
                port,
                jump_for,
                file,
                saved: shown_saved,
                offered: shown_offered,
                ..
            } => {
                assert_eq!(host_name, "db");
                assert_eq!((host.as_str(), *port), ("10.0.0.5", 2222));
                assert_eq!(jump_for.as_deref(), Some("db"));
                assert_eq!(file, "/home/me/.ssh/known_hosts");
                let shown = |key: &PublicKey| KeyFingerprint {
                    key_type: String::from("ssh-ed25519"),
                    fingerprint: fingerprint(key),
                };
                assert_eq!(shown_saved, &vec![shown(&saved)]);
                assert_eq!(shown_offered, &shown(&offered));
            }
            _ => unreachable!(),
        }
        let id = request_id(&event);
        answer(id, Decision::Once).expect("answer");
        assert_eq!(asking.await.expect("ran"), Decision::Once);
        assert!(matches!(
            answer(id, Decision::Update),
            Err(HostKeyError::NotRequested)
        ));
    }

    #[tokio::test]
    async fn a_connection_that_stops_waiting_refuses_a_late_answer() {
        let (tx, mut rx) = mpsc::channel(8);
        let changed = changed_key(22);
        let asking =
            tokio::spawn(async move { HostKeyPrompter::new(tx, "db").ask(&changed).await });
        let id = request_id(&asked(&mut rx).await);
        asking.abort();
        let _ = asking.await;
        assert!(matches!(
            answer(id, Decision::Update),
            Err(HostKeyError::NotRequested)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unanswered_question_is_a_cancel() {
        let (tx, mut rx) = mpsc::channel(8);
        let changed = changed_key(22);
        let asking =
            tokio::spawn(async move { HostKeyPrompter::new(tx, "db").ask(&changed).await });
        let id = request_id(&asked(&mut rx).await);
        tokio::time::advance(PROMPT_TIMEOUT + Duration::from_secs(1)).await;
        assert_eq!(asking.await.expect("ran"), Decision::Cancel);
        assert!(matches!(
            answer(id, Decision::Once),
            Err(HostKeyError::NotRequested)
        ));
    }

    #[tokio::test]
    async fn nobody_to_ask_is_a_cancel() {
        let (tx, rx) = mpsc::channel(8);
        drop(rx);
        let prompter = HostKeyPrompter::new(tx, "db");
        assert_eq!(prompter.ask(&changed_key(22)).await, Decision::Cancel);
    }

    #[test]
    fn a_declined_key_gets_the_headline_only() {
        assert_eq!(
            changed_key(2222).shown(Shown::Declined).to_string(),
            "Host key of 10.0.0.5 port 2222 has changed"
        );
    }

    #[test]
    fn a_key_that_keeps_changing_is_turned_down_in_short() {
        assert_eq!(
            changed_key(22).shown(Shown::KeepsChanging).to_string(),
            "Host key of 10.0.0.5 keeps changing; not connecting"
        );
    }
}
