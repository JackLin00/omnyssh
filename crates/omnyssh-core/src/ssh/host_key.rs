//! A host key that no longer matches `known_hosts`.
//!
//! The connection is turned down first, never held open mid-handshake while the
//! user decides: a server gives a login two minutes (OpenSSH's `LoginGraceTime`).
//! The key the server showed stays in the backend; frontends only ever see its
//! fingerprint.

use std::fmt;
use std::path::PathBuf;

use russh::keys::key::PublicKey;

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
    #[allow(dead_code)] // Shown to the user from Task L3; the attribute goes then.
    pub(crate) saved: Vec<PublicKey>,
    /// The key the server showed. Never leaves the backend.
    pub(crate) offered: PublicKey,
    /// The host a bastion was on the way to, when the key was met on one.
    #[allow(dead_code)] // Shown to the user from Task L3; the attribute goes then.
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

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::key::KeyPair;

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
}
