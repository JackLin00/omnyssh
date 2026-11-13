//! Key generators (`key_generators.toml`): JS code that makes a device's SSH key
//! from a shared credential and the host's MAC and revision. The frontend lists,
//! edits and runs them; renaming or deleting one follows through to the hosts
//! that name it. A login's key request arrives as `generated-key-required` and
//! is answered with `answer_generated_key`.

use omnyssh_core::config::key_generators::{
    load_key_generators, save_key_generators, KeyGenerator, KeyGenerators,
};
use omnyssh_core::config::{load_hosts, save_hosts};
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::key_gen;

use crate::dto::{KeyGeneratorDto, KeyGeneratorsDto};
use crate::error::CommandError;

/// The switch and every generator, credential included: the settings page
/// edits them.
#[tauri::command]
#[specta::specta]
pub async fn list_key_generators() -> Result<KeyGeneratorsDto, CommandError> {
    let config = blocking(|| load_key_generators().map_err(|e| format!("{e:#}"))).await?;
    Ok(KeyGeneratorsDto::from(&config))
}

/// Switch key generators on or off for every host.
#[tauri::command]
#[specta::specta]
pub async fn set_key_generators_enabled(enabled: bool) -> Result<(), CommandError> {
    update_generators(move |config| {
        config.enabled = enabled;
        Ok(())
    })
    .await
}

/// Add `generator`, or replace the one named `previous_name`. A new name moves
/// every host that named the old one over to it. The frontend reloads the hosts
/// afterwards when it renamed one.
#[tauri::command]
#[specta::specta]
pub async fn save_key_generator(
    previous_name: Option<String>,
    generator: KeyGeneratorDto,
) -> Result<(), CommandError> {
    let generator = KeyGenerator::from(generator);
    blocking(move || {
        let _guard = WRITE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = load_key_generators().map_err(|e| format!("{e:#}"))?;
        // Hosts first: if saving the generators then fails, a retry still works.
        if let Some((from, to)) = upsert(&mut config, previous_name.as_deref(), generator)? {
            let mut hosts = load_hosts().map_err(|e| e.to_string())?;
            rename_in(&mut hosts, &from, &to);
            save_hosts(&hosts).map_err(|e| e.to_string())?;
        }
        save_key_generators(&config).map_err(|e| format!("{e:#}"))
    })
    .await
}

/// Delete the generator named `name`; the hosts that named it lose it, with
/// their MAC and revision. A missing name is a no-op. The frontend reloads the
/// hosts afterwards.
#[tauri::command]
#[specta::specta]
pub async fn delete_key_generator(name: String) -> Result<(), CommandError> {
    blocking(move || {
        let _guard = WRITE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = load_key_generators().map_err(|e| format!("{e:#}"))?;
        let mut hosts = load_hosts().map_err(|e| e.to_string())?;
        clear_in(&mut hosts, &name);
        save_hosts(&hosts).map_err(|e| e.to_string())?;
        config.generators.retain(|g| g.name != name);
        save_key_generators(&config).map_err(|e| format!("{e:#}"))
    })
    .await
}

/// Answer the `generated-key-required` request `request_id` with the private
/// key text, or with why there is none. The key stays in memory only.
#[tauri::command]
#[specta::specta]
pub fn answer_generated_key(
    request_id: u64,
    private_key: Option<String>,
    error: Option<String>,
) -> Result<(), CommandError> {
    let answer = match private_key {
        Some(key) => Ok(key),
        None => Err(error.unwrap_or_else(|| "no key".to_string())),
    };
    key_gen::answer(request_id, answer).map_err(|e| CommandError {
        message: e.to_string(),
    })
}

/// The `SHA256:…` fingerprint of private key `private_key`, read as a login
/// reads it: the settings page's test. The key is not kept.
#[tauri::command]
#[specta::specta]
pub fn key_fingerprint(private_key: String) -> Result<String, CommandError> {
    key_gen::fingerprint(&private_key).map_err(|message| CommandError { message })
}

/// Puts `generator` in place of `previous` (or adds it), checked. Returns the
/// rename, if it was one.
fn upsert(
    config: &mut KeyGenerators,
    previous: Option<&str>,
    mut generator: KeyGenerator,
) -> Result<Option<(String, String)>, String> {
    generator.name = generator.name.trim().to_string();
    if generator.name.is_empty() {
        return Err("a key generator needs a name".to_string());
    }
    if !generator.credential.trim().is_empty()
        && serde_json::from_str::<serde_json::Value>(&generator.credential).is_err()
    {
        return Err("the credential is not valid JSON".to_string());
    }
    if let Some(from) = previous {
        if !config.generators.iter().any(|g| g.name == from) {
            return Err(format!("key generator '{from}' no longer exists"));
        }
    }
    let taken = config
        .generators
        .iter()
        .any(|g| g.name == generator.name && Some(g.name.as_str()) != previous);
    if taken {
        return Err(format!(
            "a key generator named '{}' already exists",
            generator.name
        ));
    }
    let renamed = previous
        .filter(|from| *from != generator.name)
        .map(|from| (from.to_string(), generator.name.clone()));
    match previous.and_then(|from| config.generators.iter().position(|g| g.name == from)) {
        Some(i) => config.generators[i] = generator,
        None => config.generators.push(generator),
    }
    Ok(renamed)
}

fn rename_in(hosts: &mut [Host], from: &str, to: &str) {
    for host in hosts {
        if host.key_generator.as_deref() == Some(from) {
            host.key_generator = Some(to.to_string());
        }
    }
}

fn clear_in(hosts: &mut [Host], name: &str) {
    for host in hosts {
        if host.key_generator.as_deref() == Some(name) {
            host.key_generator = None;
            host.device_mac = None;
            host.device_revision = None;
        }
    }
}

/// Key generator commands load, change and save whole files; two at once would
/// lose one's change.
static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CommandError {
            message: format!("key generator task failed: {e}"),
        })?
        .map_err(|message| CommandError { message })
}

/// Load the generators, apply `mutate`, and write them back.
async fn update_generators<T: Send + 'static>(
    mutate: impl FnOnce(&mut KeyGenerators) -> Result<T, String> + Send + 'static,
) -> Result<T, CommandError> {
    blocking(move || {
        let _guard = WRITE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = load_key_generators().map_err(|e| format!("{e:#}"))?;
        let out = mutate(&mut config)?;
        save_key_generators(&config).map_err(|e| format!("{e:#}"))?;
        Ok(out)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator(name: &str) -> KeyGenerator {
        KeyGenerator {
            name: name.to_string(),
            code: "code".to_string(),
            credential: r#"{"master_key":"k"}"#.to_string(),
        }
    }

    fn config() -> KeyGenerators {
        KeyGenerators {
            enabled: true,
            generators: vec![generator("a"), generator("b")],
        }
    }

    #[test]
    fn a_new_generator_is_added_and_an_edit_replaces_in_place() {
        let mut config = config();
        assert_eq!(upsert(&mut config, None, generator(" c ")), Ok(None));
        assert_eq!(config.generators[2].name, "c");

        let mut edited = generator("a");
        edited.code = "new".to_string();
        assert_eq!(upsert(&mut config, Some("a"), edited), Ok(None));
        assert_eq!(config.generators[0].code, "new");
        assert_eq!(config.generators.len(), 3);
    }

    #[test]
    fn a_rename_says_so_and_keeps_the_place() {
        let mut config = config();
        let renamed = upsert(&mut config, Some("a"), generator("z"));
        assert_eq!(renamed, Ok(Some(("a".to_string(), "z".to_string()))));
        assert_eq!(config.generators[0].name, "z");
    }

    #[test]
    fn bad_generators_are_refused() {
        let mut config = config();
        assert_eq!(
            upsert(&mut config, None, generator(" ")),
            Err("a key generator needs a name".to_string())
        );
        assert_eq!(
            upsert(&mut config, Some("a"), generator("b")),
            Err("a key generator named 'b' already exists".to_string())
        );
        let mut bad = generator("c");
        bad.credential = "{master_key: 1}".to_string();
        assert_eq!(
            upsert(&mut config, None, bad),
            Err("the credential is not valid JSON".to_string())
        );
        // No credential at all is fine.
        let mut none = generator("c");
        none.credential = "  ".to_string();
        assert_eq!(upsert(&mut config, None, none), Ok(None));
    }

    #[test]
    fn a_previous_name_that_is_gone_is_refused() {
        let mut config = config();
        assert_eq!(
            upsert(&mut config, Some("gone"), generator("c")),
            Err("key generator 'gone' no longer exists".to_string())
        );
        assert_eq!(config.generators.len(), 2);
    }

    fn host(name: &str, generator: Option<&str>) -> Host {
        Host {
            name: name.to_string(),
            key_generator: generator.map(str::to_string),
            device_mac: generator.map(|_| "30:ae:7b:eb:17:82".to_string()),
            device_revision: generator.map(|_| 2),
            ..Host::default()
        }
    }

    #[test]
    fn renames_and_deletions_reach_the_hosts_that_name_the_generator() {
        let mut hosts = vec![
            host("one", Some("a")),
            host("two", Some("b")),
            host("three", None),
        ];
        rename_in(&mut hosts, "a", "z");
        assert_eq!(hosts[0].key_generator.as_deref(), Some("z"));
        assert_eq!(hosts[1].key_generator.as_deref(), Some("b"));

        clear_in(&mut hosts, "z");
        assert_eq!(hosts[0].key_generator, None);
        assert_eq!(hosts[0].device_mac, None);
        assert_eq!(hosts[0].device_revision, None);
        assert_eq!(hosts[1].device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
    }
}
