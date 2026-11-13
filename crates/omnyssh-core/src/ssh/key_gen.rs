//! Private keys made by a key generator (`key_generators.toml`) for a host that
//! names one: JS code the desktop frontend runs on the generator's credential and
//! the host's MAC and revision. A login asks for the key with
//! [`CoreEvent::KeyRequired`] and the frontend replies through [`answer`]. Keys
//! are kept in process memory only, for as long as nothing they came from
//! changes. Without a frontend serving requests (the TUI), nothing is asked.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use russh::keys::key::KeyPair;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::config::key_generators::{load_key_generators, KeyGenerators};
use crate::event::CoreEvent;
use crate::ssh::client::Host;

/// How long a login waits for the frontend's key. The code itself is stopped
/// after 10 s there; this also covers a frontend that never answers.
pub(crate) const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// What a key depends on: any change makes another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    generator: String,
    code: String,
    credential: String,
    mac: Option<String>,
    revision: Option<u32>,
    host: String,
    hostname: String,
    user: String,
    port: u16,
}

#[derive(Default)]
struct State {
    /// Where requests go: the frontend that serves them.
    service: Option<mpsc::Sender<CoreEvent>>,
    /// Requests waiting for an answer, by id.
    pending: HashMap<u64, oneshot::Sender<Result<String, String>>>,
    last_request: u64,
    /// Generated private keys (their text), by what they were made from.
    keys: HashMap<CacheKey, String>,
}

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Makes `tx` the frontend that runs key generators: logins send it
/// [`CoreEvent::KeyRequired`] and wait for [`answer`].
pub fn serve(tx: mpsc::Sender<CoreEvent>) {
    state().service = Some(tx);
}

/// An answer that has no request to go to.
#[derive(Debug, Error)]
pub enum KeyGenError {
    /// The request was answered already, or its login stopped waiting.
    #[error("no login is waiting for this key")]
    NotRequested,
}

/// Answers the request `request_id` of a [`CoreEvent::KeyRequired`] with the
/// private key text, or why there is none.
///
/// # Errors
/// [`KeyGenError::NotRequested`] when no login waits on that request.
pub fn answer(request_id: u64, key: Result<String, String>) -> Result<(), KeyGenError> {
    let reply = state()
        .pending
        .remove(&request_id)
        .ok_or(KeyGenError::NotRequested)?;
    reply.send(key).map_err(|_| KeyGenError::NotRequested)
}

/// Reads a private key as a login would.
///
/// # Errors
/// Why it cannot be used: not private key text, or protected by a passphrase.
pub(crate) fn parse_key(text: &str) -> Result<KeyPair, String> {
    if !text.trim_start().starts_with("-----BEGIN") {
        return Err("the code did not return private key text".to_string());
    }
    russh::keys::decode_secret_key(text, None).map_err(|e| match e {
        russh::keys::Error::KeyIsEncrypted => "the key is protected by a passphrase".to_string(),
        e => format!("the key cannot be read: {e}"),
    })
}

/// The `SHA256:…` fingerprint of the public half of private key `text`, read as
/// a login reads it.
///
/// # Errors
/// Why a login could not use it.
pub fn fingerprint(text: &str) -> Result<String, String> {
    let key = parse_key(text)?;
    let public = key
        .clone_public_key()
        .map_err(|e| format!("the key cannot be read: {e}"))?;
    Ok(format!("SHA256:{}", public.fingerprint()))
}

/// The generated key a login tries first.
pub(crate) enum Generated {
    /// The host has no generator, generators are off, or nothing runs them.
    Unused,
    Key {
        generator: String,
        key: Box<KeyPair>,
    },
    /// Nothing usable came of it; the login goes on without.
    Failed { generator: String, reason: String },
}

/// The key `host`'s generator makes, from memory or asked of the frontend.
pub(crate) async fn generate(host: &Host) -> Generated {
    let Some(name) = host.key_generator.clone() else {
        return Generated::Unused;
    };
    let service = state().service.clone();
    if service.is_none() {
        return Generated::Unused;
    }
    let config = match tokio::task::spawn_blocking(load_key_generators).await {
        Ok(Ok(config)) => config,
        Ok(Err(e)) => {
            return Generated::Failed {
                generator: name,
                reason: format!("{e:#}"),
            }
        }
        Err(e) => {
            return Generated::Failed {
                generator: name,
                reason: e.to_string(),
            }
        }
    };
    generate_with(host, &config, service, ANSWER_TIMEOUT).await
}

async fn generate_with(
    host: &Host,
    config: &KeyGenerators,
    service: Option<mpsc::Sender<CoreEvent>>,
    wait: Duration,
) -> Generated {
    let (Some(name), Some(service)) = (host.key_generator.clone(), service) else {
        return Generated::Unused;
    };
    if !config.enabled {
        return Generated::Unused;
    }
    let failed = |reason: String| Generated::Failed {
        generator: name.clone(),
        reason,
    };
    let Some(generator) = config.find(&name) else {
        return failed("there is no key generator by that name".to_string());
    };
    let cache_key = CacheKey {
        generator: name.clone(),
        code: generator.code.clone(),
        credential: generator.credential.clone(),
        mac: host.device_mac.clone(),
        revision: host.device_revision,
        host: host.name.clone(),
        hostname: host.hostname.clone(),
        user: host.user.clone(),
        port: host.port,
    };
    let held = state().keys.get(&cache_key).cloned();
    let text = match held {
        Some(text) => text,
        None => {
            let event = |request_id| CoreEvent::KeyRequired {
                request_id,
                host_name: host.name.clone(),
                code: generator.code.clone(),
                credential: generator.credential.clone(),
                mac: host.device_mac.clone().unwrap_or_default(),
                revision: host.device_revision,
                address: host.hostname.clone(),
            };
            match ask(&service, event, wait).await {
                Ok(text) => text,
                Err(reason) => return failed(reason),
            }
        }
    };
    match parse_key(&text) {
        Ok(key) => {
            state().keys.insert(cache_key, text);
            Generated::Key {
                generator: name,
                key: Box::new(key),
            }
        }
        Err(reason) => failed(reason),
    }
}

/// Sends the request `event` makes of its id and waits for the answer.
async fn ask(
    service: &mpsc::Sender<CoreEvent>,
    event: impl FnOnce(u64) -> CoreEvent,
    wait: Duration,
) -> Result<String, String> {
    let (reply, answered) = oneshot::channel();
    let request_id = {
        let mut state = state();
        state.last_request += 1;
        let id = state.last_request;
        state.pending.insert(id, reply);
        id
    };
    let _pending = Pending(request_id);
    // One deadline for sending and answering: a full channel must not stretch
    // the login past `wait` either.
    let deadline = tokio::time::Instant::now() + wait;
    match tokio::time::timeout_at(deadline, service.send(event(request_id))).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => return Err("the desktop app is not running it".to_string()),
        Err(_) => return Err(format!("no key within {} s", wait.as_secs())),
    }
    match tokio::time::timeout_at(deadline, answered).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err("the request was dropped".to_string()),
        Err(_) => Err(format!("no key within {} s", wait.as_secs())),
    }
}

/// A request waiting for its answer; unlisted once the login stops waiting,
/// so a late answer is refused.
struct Pending(u64);

impl Drop for Pending {
    fn drop(&mut self) {
        state().pending.remove(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::config::key_generators::KeyGenerator;

    /// A fresh Ed25519 key as PKCS#8 PEM text.
    fn key_text() -> String {
        let key = KeyPair::generate_ed25519();
        let mut pem = Vec::new();
        russh::keys::encode_pkcs8_pem(&key, &mut pem).unwrap();
        String::from_utf8(pem).unwrap()
    }

    fn config(enabled: bool) -> KeyGenerators {
        KeyGenerators {
            enabled,
            generators: vec![KeyGenerator {
                name: "vendor-a".to_string(),
                code: "code".to_string(),
                credential: r#"{"master_key":"k"}"#.to_string(),
            }],
        }
    }

    fn host(mac: &str) -> Host {
        Host {
            name: format!("cam-{mac}"),
            hostname: "192.168.11.1".to_string(),
            key_generator: Some("vendor-a".to_string()),
            device_mac: Some(mac.to_string()),
            device_revision: Some(2),
            ..Host::default()
        }
    }

    /// A frontend answering every request with `reply`, counting them and
    /// keeping the last.
    fn frontend(
        reply: Result<String, String>,
    ) -> (
        mpsc::Sender<CoreEvent>,
        Arc<AtomicUsize>,
        Arc<Mutex<Option<CoreEvent>>>,
    ) {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(8);
        let count = Arc::new(AtomicUsize::new(0));
        let last = Arc::new(Mutex::new(None));
        let (seen, kept) = (Arc::clone(&count), Arc::clone(&last));
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let CoreEvent::KeyRequired { request_id, .. } = event {
                    seen.fetch_add(1, Ordering::SeqCst);
                    *kept.lock().unwrap() = Some(event);
                    let _ = answer(request_id, reply.clone());
                }
            }
        });
        (tx, count, last)
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[tokio::test]
    async fn a_full_channel_does_not_hold_the_login() {
        let (tx, _rx) = mpsc::channel::<CoreEvent>(1);
        tx.send(CoreEvent::Error(String::new())).await.unwrap();
        let got = generate_with(
            &host("aa:08"),
            &config(true),
            Some(tx),
            Duration::from_millis(50),
        )
        .await;
        match got {
            Generated::Failed { reason, .. } => {
                assert!(reason.starts_with("no key within"), "{reason}")
            }
            _ => panic!("expected a failure"),
        }
    }

    #[tokio::test]
    async fn the_frontend_gets_the_generator_and_the_device() {
        let (tx, _, last) = frontend(Ok(key_text()));
        let got = generate_with(&host("aa:01"), &config(true), Some(tx), WAIT).await;
        assert!(matches!(got, Generated::Key { ref generator, .. } if generator == "vendor-a"));
        let seen = last.lock().unwrap().take();
        match seen {
            Some(CoreEvent::KeyRequired {
                host_name,
                code,
                credential,
                mac,
                revision,
                address,
                ..
            }) => {
                assert_eq!(host_name, "cam-aa:01");
                assert_eq!(code, "code");
                assert_eq!(credential, r#"{"master_key":"k"}"#);
                assert_eq!(mac, "aa:01");
                assert_eq!(revision, Some(2));
                assert_eq!(address, "192.168.11.1");
            }
            other => panic!("expected KeyRequired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_key_is_made_once_until_what_it_comes_from_changes() {
        let (tx, count, _) = frontend(Ok(key_text()));
        let cfg = config(true);
        generate_with(&host("aa:02"), &cfg, Some(tx.clone()), WAIT).await;
        generate_with(&host("aa:02"), &cfg, Some(tx.clone()), WAIT).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the second login uses the held key"
        );

        let mut other = host("aa:02");
        other.device_revision = Some(3);
        generate_with(&other, &cfg, Some(tx.clone()), WAIT).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "another revision is another key"
        );

        let mut changed = cfg.clone();
        changed.generators[0].credential = r#"{"master_key":"other"}"#.to_string();
        generate_with(&host("aa:02"), &changed, Some(tx), WAIT).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "another credential is another key"
        );
    }

    #[tokio::test]
    async fn nothing_is_asked_while_switched_off_or_without_a_generator() {
        let (tx, count, _) = frontend(Ok(key_text()));
        let got = generate_with(&host("aa:03"), &config(false), Some(tx.clone()), WAIT).await;
        assert!(matches!(got, Generated::Unused));
        let mut plain = host("aa:03");
        plain.key_generator = None;
        let got = generate_with(&plain, &config(true), Some(tx), WAIT).await;
        assert!(matches!(got, Generated::Unused));
        // Without a frontend (the TUI) nothing is asked either.
        let got = generate_with(&host("aa:03"), &config(true), None, WAIT).await;
        assert!(matches!(got, Generated::Unused));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failures_say_why_and_are_not_kept() {
        let (tx, count, _) = frontend(Err("TypeError: x is undefined".to_string()));
        let got = generate_with(&host("aa:04"), &config(true), Some(tx.clone()), WAIT).await;
        assert!(
            matches!(got, Generated::Failed { ref reason, .. } if reason == "TypeError: x is undefined")
        );
        generate_with(&host("aa:04"), &config(true), Some(tx), WAIT).await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "a failure is asked again");

        let (tx, _, _) = frontend(Ok("ssh-ed25519 AAAA".to_string()));
        let got = generate_with(&host("aa:05"), &config(true), Some(tx), WAIT).await;
        assert!(
            matches!(got, Generated::Failed { ref reason, .. } if reason.contains("private key text"))
        );

        let mut unknown = host("aa:06");
        unknown.key_generator = Some("gone".to_string());
        let (tx, _, _) = frontend(Ok(key_text()));
        let got = generate_with(&unknown, &config(true), Some(tx), WAIT).await;
        assert!(
            matches!(got, Generated::Failed { ref generator, ref reason } if generator == "gone" && reason.contains("no key generator"))
        );
    }

    #[tokio::test]
    async fn a_frontend_that_never_answers_is_given_up_on() {
        let (tx, mut rx) = mpsc::channel::<CoreEvent>(8);
        let silent = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let got = generate_with(
            &host("aa:07"),
            &config(true),
            Some(tx),
            Duration::from_millis(50),
        )
        .await;
        assert!(
            matches!(got, Generated::Failed { ref reason, .. } if reason.starts_with("no key within"))
        );
        silent.abort();
    }

    /// What the vendor code returns: an OpenSSH-format Ed25519 key.
    #[test]
    fn an_openssh_ed25519_key_is_read() {
        let KeyPair::Ed25519(signing) = KeyPair::generate_ed25519() else {
            unreachable!()
        };
        let keypair = ssh_key::private::Ed25519Keypair {
            public: ssh_key::public::Ed25519PublicKey(signing.verifying_key().to_bytes()),
            private: ssh_key::private::Ed25519PrivateKey::from_bytes(&signing.to_bytes()),
        };
        let text = ssh_key::PrivateKey::from(keypair)
            .to_openssh(ssh_key::LineEnding::LF)
            .unwrap();
        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        let expected = format!(
            "SHA256:{}",
            KeyPair::Ed25519(signing)
                .clone_public_key()
                .unwrap()
                .fingerprint()
        );
        assert_eq!(fingerprint(&text), Ok(expected));
    }

    #[test]
    fn a_late_answer_is_refused() {
        assert!(matches!(
            answer(u64::MAX, Ok(String::new())),
            Err(KeyGenError::NotRequested)
        ));
    }

    #[test]
    fn fingerprints_match_the_public_key() {
        let key = KeyPair::generate_ed25519();
        let mut pem = Vec::new();
        russh::keys::encode_pkcs8_pem(&key, &mut pem).unwrap();
        let expected = format!("SHA256:{}", key.clone_public_key().unwrap().fingerprint());
        assert_eq!(fingerprint(&String::from_utf8(pem).unwrap()), Ok(expected));
        assert!(fingerprint("not a key").is_err());
        assert!(fingerprint(
            "-----BEGIN OPENSSH PRIVATE KEY-----\ngarbage\n-----END OPENSSH PRIVATE KEY-----\n"
        )
        .is_err());
    }
}
