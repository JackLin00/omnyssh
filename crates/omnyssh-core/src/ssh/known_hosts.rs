//! Host key verification against `known_hosts`, as ssh(1) does it.
//!
//! Keys live in `~/.ssh/known_hosts` on every platform, shared with OpenSSH.
//! Up to 1.1.3 the Windows build kept them in `%USERPROFILE%\ssh\known_hosts`
//! (no dot, russh-keys' own choice there); keys pinned in that file are still
//! honoured, but it is never written again.

use std::borrow::Cow;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use hmac::{Hmac, Mac};
use russh::keys::key::{self, PublicKey};
use russh::keys::known_hosts::{known_host_keys_path, learn_known_hosts_path};
use russh::keys::PublicKeyBase64;
use sha1::Sha1;

/// What `known_hosts` says about the key a server offered.
pub(crate) enum Verdict {
    /// A saved key matches.
    Known,
    /// No key of this type is saved for the host.
    Unknown,
    /// A saved key of the same type differs.
    Changed(PathBuf),
    /// The file could not be read or parsed, or there is no home to find it in.
    Unreadable(PathBuf, russh::keys::Error),
}

/// `~/.ssh/known_hosts`, the file new keys are saved to.
pub(crate) fn path() -> Option<PathBuf> {
    home().map(|home| home.join(".ssh").join("known_hosts"))
}

// `USERPROFILE` first on Windows, as russh-keys resolves its own file.
fn home() -> Option<PathBuf> {
    std::env::home_dir()
}

#[cfg(windows)]
fn legacy_path() -> Option<PathBuf> {
    home().map(|home| home.join("ssh").join("known_hosts"))
}

#[cfg(not(windows))]
fn legacy_path() -> Option<PathBuf> {
    None
}

/// The files to consult, in order.
fn files() -> Vec<PathBuf> {
    path().into_iter().chain(legacy_path()).collect()
}

/// Checks `key`, offered by `host:port`, against the saved keys.
pub(crate) fn check(host: &str, port: u16, key: &PublicKey) -> Verdict {
    check_in(&files(), host, port, key)
}

/// The first file holding a key of the offered type decides. Any matching line
/// is enough, as in ssh(1): a stale line next to the right one refuses nothing.
/// A file that cannot be opened counts as empty, as in ssh(1).
fn check_in(files: &[PathBuf], host: &str, port: u16, key: &PublicKey) -> Verdict {
    // No home: nothing to check against, and nowhere a key could be pinned.
    if files.is_empty() {
        return Verdict::Unreadable(
            PathBuf::from("~/.ssh/known_hosts"),
            russh::keys::Error::NoHomeDir,
        );
    }
    for file in files {
        let saved = match saved_keys(file, host, port) {
            Ok(saved) => saved,
            Err(e) => return Verdict::Unreadable(file.clone(), e),
        };
        if saved.contains(key) {
            return Verdict::Known;
        }
        if saved.iter().any(|k| same_type(k, key)) {
            return Verdict::Changed(file.clone());
        }
    }
    Verdict::Unknown
}

/// The keys `file` holds for `host:port`. ssh(1) writes names in lower case and
/// matches them regardless of case, so a mixed-case name is looked up both ways.
fn saved_keys(file: &Path, host: &str, port: u16) -> Result<Vec<PublicKey>, russh::keys::Error> {
    let lower = host.to_ascii_lowercase();
    let mut names = vec![host];
    if lower != host {
        names.push(&lower);
    }
    let mut keys = Vec::new();
    for name in names {
        keys.extend(
            known_host_keys_path(name, port, file)?
                .into_iter()
                .map(|(_, key)| key),
        );
    }
    Ok(keys)
}

/// Saves a key first seen on this connection (trust on first use).
pub(crate) fn learn(host: &str, port: u16, key: &PublicKey) -> Result<(), russh::keys::Error> {
    let path = path().ok_or(russh::keys::Error::NoHomeDir)?;
    learn_known_hosts_path(host, port, key, path)
}

/// Saves `key` as the one `host:port` shows now, in `~/.ssh/known_hosts`, in
/// place of the saved keys of its type. A key pinned in the Windows build's old
/// file stays there: the main file is read first, so the new key decides.
#[allow(dead_code)] // Called by the asking connection in Task L3; the attribute goes then.
pub(crate) fn update(host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    let path = path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    replace_in(&path, host, port, key).map(drop)
}

/// Pins `key` for `host:port` in `file` in place of the saved keys of its type:
/// their lines go, the rest of the file stays as it was, and `key` is added at
/// the end. A line that names other hosts too goes whole, as with
/// `ssh-keygen -R`. One write, through a temp file and a rename, so the old key
/// is never gone without the new one in its place; on unix the file keeps its
/// permissions, and a symlinked file is written through. When a removed line
/// named the host hashed (`|1|…`, `HashKnownHosts`), the new one is hashed too,
/// with a fresh salt as ssh(1) does it, so the file does not give the name away;
/// otherwise it is written plain, as `learn` writes it.
///
/// Returns how many lines were removed.
pub(crate) fn replace_in(
    file: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> std::io::Result<usize> {
    // Dotfile managers link the file: write the real one, not over the link.
    let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let old = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let entry = host_entry(host, port);
    let mut removed = 0;
    let mut hashed = false;
    let mut new = String::with_capacity(old.len() + 128);
    for line in old.split_inclusive('\n') {
        match pins_type(line, &entry, key) {
            Some(by_hash) => {
                removed += 1;
                hashed |= by_hash;
            }
            None => new.push_str(line),
        }
    }
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    // ssh(1) hashes the name in lower case.
    let written_as = if hashed {
        hashed_name(&entry.to_ascii_lowercase())?
    } else {
        entry
    };
    new.push_str(&format!(
        "{written_as} {} {}\n",
        key_type(key),
        key.public_key_base64()
    ));

    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("known_hosts");
    let tmp = file.with_file_name(format!(".{name}.omnyssh-tmp"));
    let written = (|| {
        let mut out = std::fs::File::create(&tmp)?;
        // The original's permissions before any content goes in.
        #[cfg(unix)]
        if let Ok(meta) = std::fs::metadata(&file) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        out.write_all(new.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, &file)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map(|()| removed)
}

/// How `host:port` is written in `known_hosts`, as ssh(1) writes it.
fn host_entry(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Whether `line` pins a key of `key`'s type for `entry`: `Some(hashed)` when it
/// does, `hashed` telling whether it names the host hashed. Comments, blank
/// lines, `@cert-authority` / `@revoked` lines and lines whose key does not
/// parse never do: what is not understood is left as it is.
fn pins_type(line: &str, entry: &str, key: &PublicKey) -> Option<bool> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
        return None;
    }
    let mut fields = line.split_whitespace();
    let (Some(hosts), Some(_type), Some(blob)) = (fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    let hashed = names(hosts, entry)?;
    russh::keys::parse_public_key_base64(blob)
        .is_ok_and(|saved| same_type(&saved, key))
        .then_some(hashed)
}

/// Whether a line's comma-separated host list names `entry`: `Some(hashed)` when
/// it does, `hashed` telling whether a hashed name matched. A plain name matches
/// regardless of case; a hashed one (`|1|salt|hash`) is tried with the name as
/// given and in lower case, which is how ssh(1) hashes it. Patterns (`*`, `?`,
/// `!`) never equal a name, as the check never matches them either.
fn names(list: &str, entry: &str) -> Option<bool> {
    let lower = entry.to_ascii_lowercase();
    let mut found = None;
    for name in list.split(',') {
        match name.strip_prefix("|1|") {
            Some(hashed) if hashes(hashed, entry) || hashes(hashed, &lower) => return Some(true),
            Some(_) => {}
            None if name.eq_ignore_ascii_case(entry) => found = Some(false),
            None => {}
        }
    }
    found
}

/// `name` hashed as `HashKnownHosts` writes it: `|1|salt|hash`, a fresh random
/// 20-byte salt and HMAC-SHA1(salt, name), both base64.
fn hashed_name(name: &str) -> std::io::Result<String> {
    let mut salt = [0u8; 20];
    getrandom::getrandom(&mut salt).map_err(|e| std::io::Error::other(e.to_string()))?;
    let mac = Hmac::<Sha1>::new_from_slice(&salt)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .chain_update(name.as_bytes())
        .finalize()
        .into_bytes();
    Ok(format!(
        "|1|{}|{}",
        data_encoding::BASE64.encode(&salt),
        data_encoding::BASE64.encode(&mac)
    ))
}

/// Whether `salt|hash` (both base64) is HMAC-SHA1(salt, name).
fn hashes(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (
        data_encoding::BASE64.decode(salt.as_bytes()),
        data_encoding::BASE64.decode(hash.as_bytes()),
    ) else {
        return false;
    };
    let Ok(mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.chain_update(name.as_bytes())
        .verify_slice(&hash)
        .is_ok()
}

/// The name a key type goes by in `known_hosts`. An RSA key's russh name follows
/// the signature hash it was negotiated with; the file wants the key's own.
pub(crate) fn key_type(key: &PublicKey) -> &'static str {
    match key {
        PublicKey::RSA { .. } => "ssh-rsa",
        _ => key.name(),
    }
}

/// Host key algorithms for `host:port`, those of the keys saved for it first,
/// as ssh(1) orders them: a server with several keys then shows the pinned one
/// rather than one of another type that would be taken as new.
pub(crate) fn preferred(host: &str, port: u16) -> Cow<'static, [key::Name]> {
    preferred_in(&files(), host, port)
}

fn preferred_in(files: &[PathBuf], host: &str, port: u16) -> Cow<'static, [key::Name]> {
    let default = russh::Preferred::DEFAULT.key;
    let Some(saved) = files
        .iter()
        .filter_map(|file| saved_keys(file, host, port).ok())
        .find(|saved| !saved.is_empty())
    else {
        return default;
    };
    // P-384 verifies fine but is missing from russh's list, so it is asked for
    // only where it is pinned.
    let (mut order, rest): (Vec<key::Name>, Vec<key::Name>) = default
        .iter()
        .copied()
        .chain([key::ECDSA_SHA2_NISTP384])
        .partition(|algo| saved.iter().any(|k| signs_with(k, algo)));
    order.extend(
        rest.into_iter()
            .filter(|algo| *algo != key::ECDSA_SHA2_NISTP384),
    );
    Cow::Owned(order)
}

/// An RSA key's name follows the signature hash it was negotiated with, not the
/// key itself.
fn same_type(a: &PublicKey, b: &PublicKey) -> bool {
    matches!((a, b), (PublicKey::RSA { .. }, PublicKey::RSA { .. })) || a.name() == b.name()
}

fn signs_with(key: &PublicKey, algo: &key::Name) -> bool {
    match key {
        PublicKey::RSA { .. } => *algo == key::RSA_SHA2_256 || *algo == key::RSA_SHA2_512,
        _ => key.name() == algo.0,
    }
}

/// The host as a refusal names it.
fn who(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("{host} port {port}")
    }
}

/// Shown when a saved key no longer matches. The first ':' closes the headline,
/// so a frontend that cuts there (the TUI status bar) keeps just that.
pub(crate) fn changed_message(host: &str, port: u16, file: &Path, fingerprint: &str) -> String {
    // `ssh-keygen -R` wants `[host]:port` off port 22; quoted, or zsh takes it
    // for a glob.
    let target = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    format!(
        "Host key of {who} has changed: it does not match the key saved in {file}. \
         If the server was reinstalled, check on the server that its key is {fingerprint}, \
         then remove the old one with ssh-keygen -R \"{target}\" -f \"{file}\". \
         Otherwise someone may be intercepting the connection.",
        who = who(host, port),
        file = file.display(),
    )
}

/// Shown when a `known_hosts` file cannot be used: the connection is refused
/// rather than let an unchecked key in.
pub(crate) fn unreadable_message(
    host: &str,
    port: u16,
    file: &Path,
    error: &russh::keys::Error,
) -> String {
    format!(
        "Host key of {} could not be checked: {} is unreadable ({error})",
        who(host, port),
        file.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::key::KeyPair;
    use std::io::Write;

    fn pubkey(pair: &KeyPair) -> PublicKey {
        pair.clone_public_key().expect("public key")
    }

    fn write(file: &Path, lines: &[String]) {
        let mut f = std::fs::File::create(file).expect("create");
        for line in lines {
            writeln!(f, "{line}").expect("write");
        }
    }

    fn line(host: &str, key: &PublicKey) -> String {
        use russh::keys::PublicKeyBase64;
        format!("{host} {} {}", key.name(), key.public_key_base64())
    }

    #[test]
    fn a_saved_key_is_known_and_a_different_one_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (server, other) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
        write(&file, &[line("10.0.0.5", &pubkey(&server))]);
        let files = [file.clone()];

        assert!(matches!(
            check_in(&files, "10.0.0.5", 22, &pubkey(&server)),
            Verdict::Known
        ));
        match check_in(&files, "10.0.0.5", 22, &pubkey(&other)) {
            Verdict::Changed(path) => assert_eq!(path, file),
            _ => panic!("expected a changed key"),
        }
        assert!(matches!(
            check_in(&files, "10.0.0.6", 22, &pubkey(&other)),
            Verdict::Unknown
        ));
    }

    #[test]
    fn a_stale_line_next_to_the_right_one_is_no_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (old, new) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
        write(
            &file,
            &[line("vm", &pubkey(&old)), line("vm", &pubkey(&new))],
        );
        assert!(matches!(
            check_in(&[file], "vm", 22, &pubkey(&new)),
            Verdict::Known
        ));
    }

    #[test]
    fn the_first_file_with_the_offered_type_decides() {
        let dir = tempfile::tempdir().unwrap();
        let (primary, legacy) = (dir.path().join("a"), dir.path().join("b"));
        let (server, stale) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
        let files = [primary.clone(), legacy.clone()];

        // The legacy pin still refuses a key nobody saved elsewhere...
        write(&legacy, &[line("vm", &pubkey(&stale))]);
        match check_in(&files, "vm", 22, &pubkey(&server)) {
            Verdict::Changed(path) => assert_eq!(path, legacy),
            _ => panic!("expected the legacy pin to refuse"),
        }
        // ...until the key is saved in the primary file, which is read first.
        write(&primary, &[line("vm", &pubkey(&server))]);
        assert!(matches!(
            check_in(&files, "vm", 22, &pubkey(&server)),
            Verdict::Known
        ));
        // A stale primary pin is never overruled by the legacy file.
        write(&primary, &[line("vm", &pubkey(&stale))]);
        write(&legacy, &[line("vm", &pubkey(&server))]);
        match check_in(&files, "vm", 22, &pubkey(&server)) {
            Verdict::Changed(path) => assert_eq!(path, primary),
            _ => panic!("expected the primary pin to refuse"),
        }
    }

    #[test]
    fn no_home_refuses_rather_than_trust_anything() {
        let key = pubkey(&KeyPair::generate_ed25519());
        assert!(matches!(
            check_in(&[], "vm", 22, &key),
            Verdict::Unreadable(..)
        ));
    }

    #[test]
    fn a_mixed_case_name_finds_the_pin_ssh_wrote_in_lower_case() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (server, other) = (KeyPair::generate_ed25519(), KeyPair::generate_ed25519());
        write(&file, &[line("build.corp.lan", &pubkey(&server))]);
        let files = [file.clone()];
        assert!(matches!(
            check_in(&files, "Build.Corp.lan", 22, &pubkey(&server)),
            Verdict::Known
        ));
        assert!(matches!(
            check_in(&files, "Build.Corp.lan", 22, &pubkey(&other)),
            Verdict::Changed(_)
        ));
    }

    #[test]
    fn saved_types_are_preferred() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        write(&file, &[]);
        let files = [file.clone()];
        assert_eq!(
            preferred_in(&files, "vm", 22),
            russh::Preferred::DEFAULT.key
        );

        // ssh(1) before 8.5 pinned ECDSA, which then has to come before Ed25519.
        let ecdsa = russh::keys::parse_public_key_base64(
            "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBAdX7uLfmKNNWdDCmvSEIf+RcVQX7pM+\
             X+JsRGPG88ZBnYMJCOypWfiNliHIPyo8fNivzpE4a6ZynYc8KHiEz+4=",
        )
        .expect("ecdsa key");
        write(&file, &[line("vm", &ecdsa)]);
        let order = preferred_in(&files, "vm", 22);
        assert_eq!(order.first(), Some(&key::ECDSA_SHA2_NISTP256));
        assert_eq!(order.len(), russh::Preferred::DEFAULT.key.len());
    }

    #[test]
    fn a_p384_pin_is_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let p384 = russh::keys::parse_public_key_base64(
            "AAAAE2VjZHNhLXNoYTItbmlzdHAzODQAAAAIbmlzdHAzODQAAABhBPsWebQPfTKmztyvWSqgE1HXWtAJwl6Y\
             YUx43JswHMefMvUBiOAnCS20o697vnbFr6WtNWGsTt48NyDfBtwezmZ4wyhOqDnd7kJL8MUsWd3S7E4xe5RBd\
             U39kfoUDZ2WOQ==",
        )
        .expect("p384 key");
        write(&file, &[line("vm", &p384)]);
        let order = preferred_in(&[file], "vm", 22);
        assert_eq!(order.first(), Some(&key::ECDSA_SHA2_NISTP384));
        assert_eq!(order.len(), russh::Preferred::DEFAULT.key.len() + 1);
    }

    #[test]
    fn an_rsa_pin_holds_whatever_hash_was_negotiated() {
        let rsa = || pubkey(&KeyPair::generate_rsa(2048, key::SignatureHash::SHA2_512).unwrap());
        let pinned = rsa();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        // Parsed back from the file, the key carries another hash name.
        write(&file, &[line("vm", &pinned)]);
        let files = [file.clone()];
        assert!(matches!(
            check_in(&files, "vm", 22, &pinned),
            Verdict::Known
        ));
        // Another RSA key is a changed one, not a new type to trust.
        match check_in(&files, "vm", 22, &rsa()) {
            Verdict::Changed(path) => assert_eq!(path, file),
            _ => panic!("an RSA key under another hash name slipped past the pin"),
        }
    }

    #[test]
    fn the_refusal_names_the_file_and_the_remedy() {
        let file = Path::new("/home/me/.ssh/known_hosts");
        let message = changed_message("10.0.0.5", 2222, file, "SHA256:abc");
        // The TUI status bar keeps what comes before the first ':'.
        assert_eq!(
            message.split(':').next(),
            Some("Host key of 10.0.0.5 port 2222 has changed")
        );
        assert!(message.contains("SHA256:abc"));
        assert!(
            message.contains("ssh-keygen -R \"[10.0.0.5]:2222\" -f \"/home/me/.ssh/known_hosts\"")
        );
    }

    fn ed25519() -> PublicKey {
        pubkey(&KeyPair::generate_ed25519())
    }

    fn read(file: &Path) -> String {
        std::fs::read_to_string(file).expect("read")
    }

    #[test]
    fn replacing_drops_the_old_key_and_pins_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (old, new, other) = (ed25519(), ed25519(), ed25519());
        write(
            &file,
            &[
                String::from("# my servers"),
                line("10.0.0.5", &old),
                String::new(),
                line("10.0.0.6", &other),
            ],
        );

        assert_eq!(replace_in(&file, "10.0.0.5", 22, &new).unwrap(), 1);
        assert_eq!(
            read(&file),
            format!(
                "# my servers\n\n{}\n{}\n",
                line("10.0.0.6", &other),
                line("10.0.0.5", &new)
            )
        );
        let files = [file.clone()];
        assert!(matches!(
            check_in(&files, "10.0.0.5", 22, &new),
            Verdict::Known
        ));
        assert!(matches!(
            check_in(&files, "10.0.0.6", 22, &other),
            Verdict::Known
        ));
        // Written through a temp file that is gone again.
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["known_hosts"]);
    }

    #[test]
    fn a_port_other_than_22_is_an_entry_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (on_22, old, new) = (ed25519(), ed25519(), ed25519());
        write(&file, &[line("vm", &on_22), line("[vm]:2222", &old)]);

        assert_eq!(replace_in(&file, "vm", 2222, &new).unwrap(), 1);
        assert_eq!(
            read(&file),
            format!("{}\n{}\n", line("vm", &on_22), line("[vm]:2222", &new))
        );
    }

    #[test]
    fn hashed_names_are_matched_regardless_of_case() {
        // Written by `ssh-keygen -H`: the first hashes `example.com`, the second
        // `[example.com]:2222`.
        const ON_22: &str = "|1|rF0iZ6UmrxfqFziUR+y1OAnbvK0=|y3TcuSq26U5HODCziCMs103PkEk= \
            ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKvMobXCP6wkUilydytvOK7LabasDPlGlF2H5s1Xy4s3";
        const ON_2222: &str = "|1|1Xtu7jI/QiC0TA60nMDciVu8sDU=|InwSbTSkr2Fi1Mtlvnuw1+QZx30= \
            ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKvMobXCP6wkUilydytvOK7LabasDPlGlF2H5s1Xy4s3";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        write(&file, &[ON_22.to_string(), ON_2222.to_string()]);
        let new = ed25519();

        assert_eq!(replace_in(&file, "example.org", 22, &new).unwrap(), 0);
        assert_eq!(replace_in(&file, "example.com", 2222, &new).unwrap(), 1);
        let text = read(&file);
        assert!(text.starts_with(&format!("{ON_22}\n")), "{text}");
        assert!(!text.contains(ON_2222), "{text}");

        assert_eq!(replace_in(&file, "Example.COM", 22, &new).unwrap(), 1);
        assert!(!read(&file).contains(ON_22));
    }

    #[test]
    fn a_hashed_entry_is_replaced_by_a_hashed_one() {
        // `[example.com]:2222`, hashed by `ssh-keygen -H`.
        const OLD: &str = "|1|1Xtu7jI/QiC0TA60nMDciVu8sDU=|InwSbTSkr2Fi1Mtlvnuw1+QZx30= \
            ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKvMobXCP6wkUilydytvOK7LabasDPlGlF2H5s1Xy4s3";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let new = ed25519();
        write(&file, &[OLD.to_string()]);

        assert_eq!(replace_in(&file, "Example.com", 2222, &new).unwrap(), 1);
        let text = read(&file);
        assert!(
            !text.contains("example.com") && !text.contains("Example.com"),
            "{text}"
        );
        let (hosts, rest) = text.trim_end().split_once(' ').expect("a key line");
        assert_eq!(rest, format!("ssh-ed25519 {}", new.public_key_base64()));
        assert!(
            hosts.starts_with("|1|") && !OLD.starts_with(hosts),
            "{hosts}"
        );
        // A fresh salt, and a name both this matcher and the check find.
        assert_ne!(hosts.split('|').nth(2), OLD.split('|').nth(2));
        assert_eq!(names(hosts, "[example.com]:2222"), Some(true));
        assert_eq!(names(hosts, "[example.com]:22"), None);
        let files = [file.clone()];
        assert!(matches!(
            check_in(&files, "example.com", 2222, &new),
            Verdict::Known
        ));
        assert!(matches!(
            check_in(&files, "Example.com", 2222, &new),
            Verdict::Known
        ));
    }

    #[test]
    fn a_plain_entry_is_replaced_by_a_plain_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (old, new) = (ed25519(), ed25519());
        write(&file, &[line("[vm]:2222", &old)]);

        assert_eq!(replace_in(&file, "vm", 2222, &new).unwrap(), 1);
        assert_eq!(read(&file), format!("{}\n", line("[vm]:2222", &new)));
    }

    #[test]
    fn a_line_naming_several_hosts_goes_whole_as_with_ssh_keygen() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (old, new) = (ed25519(), ed25519());
        write(&file, &[line("vm,10.0.0.7", &old)]);

        assert_eq!(replace_in(&file, "10.0.0.7", 22, &new).unwrap(), 1);
        assert_eq!(read(&file), format!("{}\n", line("10.0.0.7", &new)));
    }

    #[test]
    fn other_types_markers_and_unreadable_lines_stay() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let ecdsa = russh::keys::parse_public_key_base64(
            "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBAdX7uLfmKNNWdDCmvSEIf+RcVQX7pM+\
             X+JsRGPG88ZBnYMJCOypWfiNliHIPyo8fNivzpE4a6ZynYc8KHiEz+4=",
        )
        .expect("ecdsa key");
        let (old, new) = (ed25519(), ed25519());
        let revoked = format!("@revoked {}", line("vm", &old));
        let garbage = String::from("vm ssh-ed25519 not-a-key");
        write(
            &file,
            &[
                line("vm", &ecdsa),
                line("vm", &old),
                revoked.clone(),
                garbage.clone(),
            ],
        );

        assert_eq!(replace_in(&file, "vm", 22, &new).unwrap(), 1);
        assert_eq!(
            read(&file),
            format!(
                "{}\n{revoked}\n{garbage}\n{}\n",
                line("vm", &ecdsa),
                line("vm", &new)
            )
        );
    }

    #[test]
    fn line_endings_and_a_missing_last_newline_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (other, old, new) = (ed25519(), ed25519(), ed25519());

        std::fs::write(
            &file,
            format!("{}\r\n{}\r\n", line("a", &other), line("vm", &old)),
        )
        .unwrap();
        assert_eq!(replace_in(&file, "vm", 22, &new).unwrap(), 1);
        assert_eq!(
            read(&file),
            format!("{}\r\n{}\n", line("a", &other), line("vm", &new))
        );

        std::fs::write(&file, line("a", &other)).unwrap();
        assert_eq!(replace_in(&file, "vm", 22, &new).unwrap(), 0);
        assert_eq!(
            read(&file),
            format!("{}\n{}\n", line("a", &other), line("vm", &new))
        );
    }

    #[test]
    fn a_missing_file_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".ssh").join("known_hosts");
        let new = ed25519();
        assert_eq!(replace_in(&file, "vm", 22, &new).unwrap(), 0);
        assert_eq!(read(&file), format!("{}\n", line("vm", &new)));
    }

    #[test]
    fn an_rsa_key_is_matched_by_key_and_written_as_ssh_rsa() {
        let rsa = || pubkey(&KeyPair::generate_rsa(2048, key::SignatureHash::SHA2_512).unwrap());
        let (old, new) = (rsa(), rsa());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        // As russh's own TOFU writes it: under the hash name.
        write(&file, &[line("vm", &old)]);

        assert_eq!(replace_in(&file, "vm", 22, &new).unwrap(), 1);
        // `PublicKeyBase64` comes in with `use super::*` (Step 4 imports it).
        assert_eq!(
            read(&file),
            format!("vm ssh-rsa {}\n", new.public_key_base64())
        );
        assert!(matches!(check_in(&[file], "vm", 22, &new), Verdict::Known));
    }

    #[test]
    fn a_new_key_in_the_main_file_overrules_a_legacy_pin() {
        let dir = tempfile::tempdir().unwrap();
        let (primary, legacy) = (dir.path().join("a"), dir.path().join("b"));
        let (old, new) = (ed25519(), ed25519());
        write(&legacy, &[line("vm", &old)]);
        let files = [primary.clone(), legacy.clone()];
        assert!(matches!(
            check_in(&files, "vm", 22, &new),
            Verdict::Changed { .. }
        ));

        assert_eq!(replace_in(&primary, "vm", 22, &new).unwrap(), 0);
        assert_eq!(
            read(&legacy),
            format!("{}\n", line("vm", &old)),
            "the legacy file is left alone"
        );
        assert!(matches!(check_in(&files, "vm", 22, &new), Verdict::Known));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_hosts");
        let (old, new) = (ed25519(), ed25519());
        for mode in [0o600, 0o644] {
            write(&file, &[line("vm", &old)]);
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            replace_in(&file, "vm", 22, &new).unwrap();
            let kept = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(kept, mode);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let (real, link) = (dir.path().join("real"), dir.path().join("known_hosts"));
        let (old, new) = (ed25519(), ed25519());
        write(&real, &[line("vm", &old)]);
        std::os::unix::fs::symlink(&real, &link).unwrap();

        replace_in(&link, "vm", 22, &new).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(read(&real), format!("{}\n", line("vm", &new)));
    }

    /// The old file is where russh-keys itself pins keys on Windows.
    #[cfg(windows)]
    #[test]
    fn the_legacy_file_is_the_one_russh_wrote() {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("USERPROFILE", home.path());
        let key = pubkey(&KeyPair::generate_ed25519());
        russh::keys::known_hosts::learn_known_hosts("vm", 22, &key).expect("learn");
        let legacy = legacy_path().expect("legacy path");
        assert!(legacy.is_file(), "{} was not written", legacy.display());
        assert_eq!(path(), Some(home.path().join(".ssh").join("known_hosts")));
        assert!(matches!(check("vm", 22, &key), Verdict::Known));
    }
}
