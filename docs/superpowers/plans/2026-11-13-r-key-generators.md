# R. 密钥生成器 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让某类设备用"共用凭证 + 设备 MAC/revision"由用户写的 JS 代码算出各自的 SSH 私钥，连接时自动用它登录：设置页有总开关（默认关）和生成器管理，主机表单可选生成器并填 MAC、revision；生成的密钥先试，任何失败都继续原有登录方式。

**Architecture:**
- **core**：`config/key_generators.rs` 读写 `key_generators.toml`（总开关 + 生成器：名字、代码、凭证 JSON 文本，0600 写入）；`Host` 多 `key_generator`、`device_mac`、`device_revision`。`ssh/key_gen.rs` 管请求和内存缓存：登录时（`session.rs` 的 `finish_auth` 开头）对设了生成器的主机发 `CoreEvent::KeyRequired`，等前端 `answer` 回私钥文本，解析后先试这把密钥；不成功就照常走 agent → 密钥文件 → 默认密钥 → 密码。只有注册了前端（`key_gen::serve`，GUI 启动时注册）才会请求，TUI 不注册、不请求。
- **GUI 后端**：`commands/key_gen.rs`（列表、开关、保存/改名、删除、应答、算指纹），`generated-key-required` 事件，`HostDto`/`HostInputDto` 加生成器字段（`keyGen` 省略 = 保持原值）。
- **前端**：`lib/keyGen.ts` 在独立 Web Worker 里运行代码并应答登录；设置页 `KeyGeneratorSettings.svelte` + `KeyGeneratorEditor.svelte`（含测试区）；主机表单字段和卡片上的 `gen` 标签。

**Tech Stack:** Rust（russh 0.46 / russh-keys 0.46、tokio、serde/toml、tauri-specta）、Svelte 5（runes）、TypeScript、Vitest、Playwright。

**Spec:** `docs/superpowers/specs/2026-11-13-key-generators-design.md`

**命令约定（沙箱限制，务必遵守）：**
- 前台命令如果 60–100 秒没有输出会被沙箱杀掉。`cargo` 和前端命令都按各步骤给出的写法用 `nohup sh -c '…; echo EXIT=$?' > /tmp/r.log 2>&1 &` 放到后台，再反复 `tail -20 /tmp/r.log` 直到出现 `EXIT=`。
- 不要设置 `CARGO_TARGET_DIR`。
- **补丁的用法：** 每个代码步骤给出一个补丁。用 Write 工具把它原样存到指定的 `/tmp/…` 文件（不要重排、不要改空格），然后在仓库根目录 `git apply --check <文件> && git apply <文件>`。`--check` 失败就停下来报告，不要手工"修"补丁。Task 3、4 的补丁里 `bindings.ts` 有两行以空格结尾（生成器的输出），`git apply` 会提示 `trailing whitespace`，这是正常的。
- `crates/omnyssh-gui/ui/src/lib/bindings.ts` 是生成的文件，补丁里已经包含更新后的内容；`cargo test -p omnyssh-gui` 里的 `committed_bindings_are_in_sync` 会检查它和 Rust 一致。

---

## 实现中确定的设计决定

> 写这份计划时，整套改动已经在一个临时分支上完整实现并验证过：`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全部通过；前端 `npm run check`（0 错误 0 警告）、`npx vitest run`（509 个）、全量 Playwright（118 个，原有 112 个加新增 6 个）都通过。之后从 spec 提交重新开一个分支，按下面 7 个任务逐个应用补丁验证：每个测试补丁之后相关测试失败（失败情况写在各任务里），每个实现补丁之后通过，每个任务结束时的代码和临时分支逐字相同。

1. **登录顺序**：生成的密钥排在最前（`finish_auth` 开头、SSH agent 之前），因为它就是这台设备该用的密钥；agent 里的密钥先试会消耗服务器的 `MaxAuthTries`。之后的流程一行不变。
2. **什么时候不请求**（返回 `Generated::Unused`，和现在完全一样）：主机没设生成器；总开关关闭；没有前端注册（TUI）。只有设了生成器的主机才会去读 `key_generators.toml`。
3. **失败只跳过**：配置读不出来、生成器名不存在、前端 15 秒没回答、代码报错/超时、返回的不是私钥文本、私钥带密码、服务器不接受，都只是记一条原因然后照常登录。只有整个登录失败时，原因才附在错误后面：`SSH authentication failed for cam (key generator 'vendor-a': the server did not accept its key)`。只附在两处"authentication failed"的消息上；需要密码或密钥口令的错误（`NoPassword`、`PassphraseRequired`）会触发前端提示，不改它们。服务器在验证时不回应（`Stalled`）仍然中止登录，和现有密钥一样。
4. **请求里直接带代码和凭证**（`KeyRequired { code, credential, mac, revision, address }`），前端不需要预先加载生成器列表就能回答。
5. **缓存**：键是"生成器名 + 代码 + 凭证 + MAC + revision + 主机名/地址/用户/端口"，任何一项变了就重新生成；只缓存解析成功的私钥，失败下次重试（改好代码不用重启）。只在本次运行的内存里。
6. **私钥格式**：用 `russh::keys::decode_secret_key` 解析，支持 OpenSSH（`-----BEGIN OPENSSH PRIVATE KEY-----`，你们代码的格式）和 PKCS#8/PEM；测试覆盖了 OpenSSH Ed25519。
7. **`key_generators.toml` 以 0600 创建**（临时文件一创建就是 0600，再改名覆盖），凭证里的 `master_key` 不会有一刻对其他用户可读。
8. **主机保存**：`HostInputDto.keyGen` 省略表示保持原值（和 `uploadProtocol` 一样），所以总开关关闭时表单不显示这些字段、保存也不会清掉它们；`keyGen.generator` 为空时连同 MAC、revision 一起清掉。复制主机时没给 `keyGen` 就沿用源主机的。TUI 编辑主机时三个字段原样保留。
9. **改名、删除生成器**由后端一并更新 `hosts.toml` 里引用它的主机（改名跟着改；删除清掉三个字段），前端随后 `reload_hosts`。
10. **Web Worker**：每次运行新建、结束或 10 秒超时 `terminate()`；代码加载就报错（语法错误）走 `onerror`。凭证在主线程先 `JSON.parse`，不合法就不启动 Worker。没填 revision 时 `deviceInfo` 里不放这个键，让代码用自己的默认值。
11. **测试区的指纹**由后端 `key_fingerprint` 用登录时同一套解析计算，和代码返回的 `fingerprint` 比较时忽略 `SHA256:` 前缀和 base64 末尾的 `=`；不一致时额外提示一行。私钥本身不显示。
12. **主机表单**：生成器下拉框里，主机引用了但列表里已经没有的名字仍然保留为可选项，避免打开表单再保存时悄悄清掉。卡片标签 `gen` 只在总开关打开时显示。
13. **`KeyRequired` → `generated-key-required` 的映射**写在 `bridge.rs` 的事件转发循环里（和 `PasswordRequired` 一样），那里没有可单测的纯函数，所以没有单独的单元测试；它由 Task 2 的端到端测试（核心发出请求）和 Task 5 的 e2e（前端收到事件并应答）两头覆盖，Task 8 的手动验证会走通整条链路。
14. **TUI** 万一收到 `KeyRequired`（正常不会，它不注册），立即回答"不支持"，登录照常继续。

---

## 文件结构

| 文件 | 任务 | 职责 |
|---|---|---|
| `crates/omnyssh-core/src/config/key_generators.rs`（新建）、`config/mod.rs`、`utils/platform.rs` | 1 | 读写 `key_generators.toml` |
| `crates/omnyssh-core/src/ssh/client.rs` | 1 | `Host` 的三个新字段 |
| `crates/omnyssh/src/app/host.rs`、`crates/omnyssh-gui/src/dto.rs` | 1 | 补上结构体字段；TUI 编辑时保留 |
| `crates/omnyssh-core/src/ssh/key_gen.rs`（新建）、`ssh/mod.rs`、`event.rs` | 2 | 请求、应答、缓存、指纹 |
| `crates/omnyssh-core/src/ssh/session.rs` | 2 | 登录时先试生成的密钥 |
| `crates/omnyssh/src/app/mod.rs` | 2 | TUI 应答 `KeyRequired` |
| `crates/omnyssh-core/tests/key_gen_login.rs`（新建） | 2 | 进程内 SSH 服务器的端到端测试 |
| `crates/omnyssh-gui/src/dto.rs`、`commands/hosts.rs` | 3 | 主机 DTO 字段、保存/复制时保留 |
| `crates/omnyssh-gui/src/commands/key_gen.rs`（新建）、`commands/mod.rs`、`dto.rs`、`events.rs`、`bridge.rs`、`main.rs` | 4 | 生成器命令、事件、启动时注册 |
| `crates/omnyssh-gui/ui/src/lib/bindings.ts` | 3、4 | 生成的绑定 |
| `crates/omnyssh-gui/ui/src/lib/keyGen.ts`（新建）+ 测试、`ipc/commands.ts`、`ipc/subscribe.ts` | 5 | Worker 运行器、应答登录 |
| `crates/omnyssh-gui/ui/e2e/key-generators.spec.ts`（新建） | 5、6、7 | e2e 用例（每个任务加几个） |
| `crates/omnyssh-gui/ui/src/lib/stores/keyGenerators.ts`、`screens/keyGenForm.ts`（+ 测试）、`KeyGeneratorEditor.svelte`、`KeyGeneratorSettings.svelte`、`Settings.svelte` | 6 | 设置页 |
| `crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts`（+ 测试）、`HostEditor.svelte`、`Dashboard.svelte` | 7 | 主机表单字段、卡片标签 |

---

### Task 1: 配置文件和主机字段

**Files:** Create `crates/omnyssh-core/src/config/key_generators.rs`；Modify `config/mod.rs`、`utils/platform.rs`、`ssh/client.rs`、`crates/omnyssh/src/app/host.rs`、`crates/omnyssh-gui/src/dto.rs`

单元测试写在同一个文件里（Rust 的惯例），所以这个任务只有一个补丁：读写往返、文件不存在、字段默认值、坏文件报错带文件名、0600 权限（含残留的 0644 临时文件），以及 `Host` 新字段不设时不写进 `hosts.toml`。`dto.rs` 和 TUI 的 `host.rs` 只是补上结构体新字段，否则编译不过。

- [ ] **Step 1: 应用补丁**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r1-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r1-impl.patch && git apply /tmp/r1-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/config/key_generators.rs b/crates/omnyssh-core/src/config/key_generators.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-core/src/config/key_generators.rs
@@ -0,0 +1,183 @@
+//! Key generators: JS code that turns a shared credential and a device's MAC
+//! and revision into that device's SSH private key, in
+//! `~/.config/omnyssh/key_generators.toml`. Off unless `enabled`. The credential
+//! holds a master key, so the file is written owner-only from the first byte.
+
+use std::path::Path;
+
+use anyhow::Context;
+use serde::{Deserialize, Serialize};
+
+use crate::utils::platform;
+
+/// One generator: the code defining `generateKey(deviceInfo, address)`, and the
+/// credential (JSON text) every device using it shares.
+#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
+pub struct KeyGenerator {
+    pub name: String,
+    #[serde(default)]
+    pub code: String,
+    /// JSON text; empty for none.
+    #[serde(default)]
+    pub credential: String,
+}
+
+/// The whole file: the switch and the generators.
+#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
+pub struct KeyGenerators {
+    /// Nothing is generated while this is off, whatever the hosts say.
+    #[serde(default)]
+    pub enabled: bool,
+    #[serde(default)]
+    pub generators: Vec<KeyGenerator>,
+}
+
+impl KeyGenerators {
+    /// The generator named `name`.
+    pub fn find(&self, name: &str) -> Option<&KeyGenerator> {
+        self.generators.iter().find(|g| g.name == name)
+    }
+}
+
+/// Loads the key generators. A missing file is switched off, with none.
+///
+/// # Errors
+/// Returns an error if the file exists but cannot be read or parsed.
+pub fn load_key_generators() -> anyhow::Result<KeyGenerators> {
+    let path =
+        platform::key_generators_config_path().context("Cannot determine key generators path")?;
+    load_from(&path)
+}
+
+/// Persists the key generators.
+///
+/// # Errors
+/// Returns an error if the directory cannot be created or the file cannot be written.
+pub fn save_key_generators(config: &KeyGenerators) -> anyhow::Result<()> {
+    let path =
+        platform::key_generators_config_path().context("Cannot determine key generators path")?;
+    if let Some(dir) = path.parent() {
+        std::fs::create_dir_all(dir)
+            .with_context(|| format!("Failed to create directory {}", dir.display()))?;
+    }
+    save_to(&path, config)
+}
+
+fn load_from(path: &Path) -> anyhow::Result<KeyGenerators> {
+    if !path.exists() {
+        return Ok(KeyGenerators::default());
+    }
+    let content = std::fs::read_to_string(path)
+        .with_context(|| format!("Failed to read {}", path.display()))?;
+    toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
+}
+
+fn save_to(path: &Path, config: &KeyGenerators) -> anyhow::Result<()> {
+    let content = toml::to_string_pretty(config).context("Failed to serialise key generators")?;
+    // A temp file renamed over the old one, so an interrupted write never leaves a
+    // corrupt file; created owner-only, so the credential is never readable by others.
+    let tmp_path = path.with_extension("toml.tmp");
+    write_private(&tmp_path, content.as_bytes())
+        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
+    if let Err(e) = std::fs::rename(&tmp_path, path) {
+        let _ = std::fs::remove_file(&tmp_path);
+        return Err(e).with_context(|| {
+            format!(
+                "Failed to rename {} to {}",
+                tmp_path.display(),
+                path.display()
+            )
+        });
+    }
+    Ok(())
+}
+
+#[cfg(unix)]
+fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
+    use std::io::Write;
+    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
+    let mut file = std::fs::OpenOptions::new()
+        .write(true)
+        .create(true)
+        .truncate(true)
+        .mode(0o600)
+        .open(path)?;
+    // A temp file left by an earlier crash keeps its old mode: set it again.
+    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
+    file.write_all(bytes)
+}
+
+#[cfg(not(unix))]
+fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
+    std::fs::write(path, bytes)
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn config() -> KeyGenerators {
+        KeyGenerators {
+            enabled: true,
+            generators: vec![KeyGenerator {
+                name: "vendor-a".to_string(),
+                code: "async function generateKey(d, a) {\n  return {};\n}\n".to_string(),
+                credential: r#"{"master_key":"00ff"}"#.to_string(),
+            }],
+        }
+    }
+
+    #[test]
+    fn saved_generators_read_back_unchanged() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("key_generators.toml");
+        save_to(&path, &config()).unwrap();
+        assert_eq!(load_from(&path).unwrap(), config());
+    }
+
+    #[test]
+    fn a_missing_file_is_switched_off_with_none() {
+        let dir = tempfile::tempdir().unwrap();
+        let loaded = load_from(&dir.path().join("key_generators.toml")).unwrap();
+        assert_eq!(loaded, KeyGenerators::default());
+        assert!(!loaded.enabled);
+    }
+
+    #[test]
+    fn omitted_fields_take_their_defaults() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("key_generators.toml");
+        std::fs::write(&path, "[[generators]]\nname = \"g\"\n").unwrap();
+        let loaded = load_from(&path).unwrap();
+        assert!(!loaded.enabled);
+        assert_eq!(loaded.find("g").unwrap().code, "");
+        assert_eq!(loaded.find("g").unwrap().credential, "");
+        assert!(loaded.find("other").is_none());
+    }
+
+    #[test]
+    fn a_malformed_file_is_an_error_naming_it() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("key_generators.toml");
+        std::fs::write(&path, "generators = 1").unwrap();
+        assert!(format!("{:#}", load_from(&path).unwrap_err()).contains("key_generators.toml"));
+    }
+
+    #[cfg(unix)]
+    #[test]
+    fn the_file_is_owner_only() {
+        use std::os::unix::fs::PermissionsExt;
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("key_generators.toml");
+        // A stale temp file from a crash, readable by everyone.
+        std::fs::write(path.with_extension("toml.tmp"), "").unwrap();
+        std::fs::set_permissions(
+            path.with_extension("toml.tmp"),
+            std::fs::Permissions::from_mode(0o644),
+        )
+        .unwrap();
+        save_to(&path, &config()).unwrap();
+        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
+        assert_eq!(mode & 0o777, 0o600);
+    }
+}
diff --git a/crates/omnyssh-core/src/config/mod.rs b/crates/omnyssh-core/src/config/mod.rs
--- a/crates/omnyssh-core/src/config/mod.rs
+++ b/crates/omnyssh-core/src/config/mod.rs
@@ -5,11 +5,13 @@
 //! - [`snippets`]    — `~/.config/omnyssh/snippets.toml`
 //! - [`serial_devices`] — `~/.config/omnyssh/serial.toml`
 //! - [`quick_commands`] — `~/.config/omnyssh/quick_commands.toml`
+//! - [`key_generators`] — `~/.config/omnyssh/key_generators.toml`
 //!
 //! Top-level functions in this module handle loading and persisting the
 //! host list (`hosts.toml`).
 
 pub mod app_config;
+pub mod key_generators;
 pub mod quick_commands;
 pub mod serial_devices;
 pub mod snippets;
diff --git a/crates/omnyssh-core/src/ssh/client.rs b/crates/omnyssh-core/src/ssh/client.rs
--- a/crates/omnyssh-core/src/ssh/client.rs
+++ b/crates/omnyssh-core/src/ssh/client.rs
@@ -127,6 +127,16 @@
     /// Where SFTP uploads go: `~`, `~/dir` or an absolute path. Unset means `~`.
     #[serde(skip_serializing_if = "Option::is_none")]
     pub upload_dir: Option<String>,
+    /// The key generator (`key_generators.toml`) whose key this host is tried
+    /// with first, while generators are switched on.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub key_generator: Option<String>,
+    /// The device's MAC, handed to the key generator as it is.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub device_mac: Option<String>,
+    /// The device's revision for the key generator; unset leaves it to the code.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub device_revision: Option<u32>,
 
     // -----------------------------------------------------------------------
     // Auto SSH Key Setup metadata
@@ -184,6 +194,9 @@
             forward_agent: false,
             upload_protocol: HostUploadProtocol::default(),
             upload_dir: None,
+            key_generator: None,
+            device_mac: None,
+            device_revision: None,
             key_setup_date: None,
             password_auth_disabled: None,
         }
@@ -269,4 +282,26 @@
         assert_eq!(read.monitoring, MonitorMode::TcpPort);
         assert_eq!(read.monitor_port, Some(8443));
     }
-}
+
+    #[test]
+    fn key_generator_fields_stay_out_until_set_and_read_back() {
+        let plain: Host = toml::from_str("name = \"web\"\nhostname = \"10.0.0.1\"\n").unwrap();
+        assert_eq!(plain.key_generator, None);
+        let written = toml::to_string(&plain).expect("serialize");
+        assert!(!written.contains("key_generator"), "{written}");
+        assert!(!written.contains("device_"), "{written}");
+
+        let host = Host {
+            name: String::from("cam"),
+            hostname: String::from("192.168.11.1"),
+            key_generator: Some(String::from("vendor-a")),
+            device_mac: Some(String::from("30:ae:7b:eb:17:82")),
+            device_revision: Some(2),
+            ..Host::default()
+        };
+        let read: Host = toml::from_str(&toml::to_string(&host).unwrap()).unwrap();
+        assert_eq!(read.key_generator.as_deref(), Some("vendor-a"));
+        assert_eq!(read.device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
+        assert_eq!(read.device_revision, Some(2));
+    }
+}
diff --git a/crates/omnyssh-core/src/utils/platform.rs b/crates/omnyssh-core/src/utils/platform.rs
--- a/crates/omnyssh-core/src/utils/platform.rs
+++ b/crates/omnyssh-core/src/utils/platform.rs
@@ -51,6 +51,11 @@
     app_config_dir().map(|d| d.join("quick_commands.toml"))
 }
 
+/// Returns the path to the key generators file.
+pub fn key_generators_config_path() -> Option<PathBuf> {
+    app_config_dir().map(|d| d.join("key_generators.toml"))
+}
+
 /// Removes rolling log files in `log_dir` older than `max_age_days`.
 ///
 /// Best-effort and fault-tolerant: a missing directory, an unreadable entry,
diff --git a/crates/omnyssh-gui/src/dto.rs b/crates/omnyssh-gui/src/dto.rs
--- a/crates/omnyssh-gui/src/dto.rs
+++ b/crates/omnyssh-gui/src/dto.rs
@@ -678,6 +678,10 @@
             forward_agent: dto.forward_agent,
             upload_protocol: dto.upload_protocol.map(Into::into).unwrap_or_default(),
             upload_dir: non_empty(dto.upload_dir.map(|d| d.trim().to_string())),
+            // Set by `save_host` from the payload's `keyGen`, or carried over.
+            key_generator: None,
+            device_mac: None,
+            device_revision: None,
             key_setup_date: None,
             password_auth_disabled: None,
         }
diff --git a/crates/omnyssh/src/app/host.rs b/crates/omnyssh/src/app/host.rs
--- a/crates/omnyssh/src/app/host.rs
+++ b/crates/omnyssh/src/app/host.rs
@@ -295,6 +295,9 @@
             forward_agent,
             upload_protocol: Default::default(),
             upload_dir: None,
+            key_generator: None,
+            device_mac: None,
+            device_revision: None,
             key_setup_date: None,
             password_auth_disabled: None,
         })
@@ -579,10 +582,13 @@
                         // Nor has IdentitiesOnly, and losing it brings back a login
                         // that offers every agent key.
                         host.identities_only = old_host.is_some_and(|h| h.identities_only);
-                        // Nor have the desktop's upload settings.
+                        // Nor have the desktop's upload and key generator settings.
                         if let Some(old) = old_host {
                             host.upload_protocol = old.upload_protocol;
                             host.upload_dir = old.upload_dir.clone();
+                            host.key_generator = old.key_generator.clone();
+                            host.device_mac = old.device_mac.clone();
+                            host.device_revision = old.device_revision;
                         }
 
                         // An import is adopted under the name it was imported by; a copy
```

- [ ] **Step 2: 跑测试和编译**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib key_generator 2>&1 | grep -E "test result|^error"; cargo build --workspace 2>&1 | grep -E "^error" | head -3; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: `test result: ok. 6 passed`；`cargo build` 没有输出 `error`。

- [ ] **Step 3: Commit**

```bash
git add crates/omnyssh-core crates/omnyssh/src/app/host.rs crates/omnyssh-gui/src/dto.rs
git commit -m "feat(core): key_generators.toml and the host's key generator fields"
```

---

### Task 2: 登录时先试生成的密钥

**Files:** Create `crates/omnyssh-core/src/ssh/key_gen.rs`、`crates/omnyssh-core/tests/key_gen_login.rs`；Modify `ssh/mod.rs`、`event.rs`、`ssh/session.rs`、`crates/omnyssh/src/app/mod.rs`

端到端测试起一个只接受某把公钥或密码的进程内 SSH 服务器，再注册一个按 MAC 应答的假前端（`good` 给服务器认的私钥，`wrong` 给另一把，其它返回错误）：生成的密钥能登录；密钥被拒或生成失败时改用密码登录；全都失败时错误里有生成器的原因；主机没设生成器时不用它。`key_gen.rs` 自带单元测试：请求内容、缓存与失效、关闭/无生成器/无前端时不请求、失败不缓存、前端不回答时放弃、迟到的应答被拒、OpenSSH 和 PEM 私钥的指纹。

- [ ] **Step 1: 先加端到端测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r2-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r2-test.patch && git apply /tmp/r2-test.patch`。

```diff
diff --git a/crates/omnyssh-core/tests/key_gen_login.rs b/crates/omnyssh-core/tests/key_gen_login.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-core/tests/key_gen_login.rs
@@ -0,0 +1,186 @@
+//! Logging in with a key a key generator makes: `SshSession::connect` against an
+//! in-process SSH server that takes one public key, or a password. A stand-in
+//! frontend answers the key requests by the device's MAC.
+
+use std::net::SocketAddr;
+use std::sync::{Arc, Once, OnceLock};
+use std::time::Duration;
+
+use russh::keys::key::{KeyPair, PublicKey};
+use russh::server::{self, Auth};
+use tokio::net::TcpListener;
+use tokio::sync::mpsc;
+
+use omnyssh_core::config::key_generators::{save_key_generators, KeyGenerator, KeyGenerators};
+use omnyssh_core::event::CoreEvent;
+use omnyssh_core::ssh::client::Host;
+use omnyssh_core::ssh::key_gen;
+use omnyssh_core::ssh::session::SshSession;
+
+const PASSWORD: &str = "keygen-test";
+
+/// The key the server takes, as the generator returns it for MAC `good`.
+fn device_key() -> &'static KeyPair {
+    static KEY: OnceLock<KeyPair> = OnceLock::new();
+    KEY.get_or_init(KeyPair::generate_ed25519)
+}
+
+fn pem(key: &KeyPair) -> String {
+    let mut text = Vec::new();
+    russh::keys::encode_pkcs8_pem(key, &mut text).unwrap();
+    String::from_utf8(text).unwrap()
+}
+
+/// A temporary home with generators switched on and a frontend serving them:
+/// MAC `good` gets the server's key, `wrong` another key, `broken` an error.
+fn setup() {
+    static ONCE: Once = Once::new();
+    ONCE.call_once(|| {
+        let home = tempfile::tempdir().expect("tempdir").keep();
+        std::env::set_var("HOME", &home);
+        std::env::remove_var("XDG_CONFIG_HOME");
+        std::env::remove_var("SSH_AUTH_SOCK");
+        save_key_generators(&KeyGenerators {
+            enabled: true,
+            generators: vec![KeyGenerator {
+                name: "vendor-a".to_string(),
+                code: "async function generateKey() {}".to_string(),
+                credential: r#"{"master_key":"k"}"#.to_string(),
+            }],
+        })
+        .expect("save generators");
+
+        let (tx, mut rx) = mpsc::channel::<CoreEvent>(16);
+        key_gen::serve(tx);
+        let good = pem(device_key());
+        let wrong = pem(&KeyPair::generate_ed25519());
+        // A thread of its own: each test has its own runtime.
+        std::thread::spawn(move || {
+            while let Some(event) = rx.blocking_recv() {
+                if let CoreEvent::KeyRequired {
+                    request_id, mac, ..
+                } = event
+                {
+                    let reply = match mac.as_str() {
+                        "good" => Ok(good.clone()),
+                        "wrong" => Ok(wrong.clone()),
+                        _ => Err("TypeError: master_key is undefined".to_string()),
+                    };
+                    let _ = key_gen::answer(request_id, reply);
+                }
+            }
+        });
+    });
+}
+
+#[derive(Clone)]
+struct Server;
+
+#[async_trait::async_trait]
+impl server::Handler for Server {
+    type Error = russh::Error;
+
+    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
+        Ok(if password == PASSWORD {
+            Auth::Accept
+        } else {
+            Auth::Reject {
+                proceed_with_methods: None,
+            }
+        })
+    }
+
+    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
+        Ok(if *key == device_key().clone_public_key().unwrap() {
+            Auth::Accept
+        } else {
+            Auth::Reject {
+                proceed_with_methods: None,
+            }
+        })
+    }
+}
+
+async fn serve() -> SocketAddr {
+    let config = Arc::new(server::Config {
+        keys: vec![KeyPair::generate_ed25519()],
+        auth_rejection_time: Duration::ZERO,
+        auth_rejection_time_initial: Some(Duration::ZERO),
+        ..Default::default()
+    });
+    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
+    let addr = listener.local_addr().expect("addr");
+    tokio::spawn(async move {
+        while let Ok((socket, _)) = listener.accept().await {
+            let config = Arc::clone(&config);
+            tokio::spawn(async move {
+                if let Ok(running) = server::run_stream(config, socket, Server).await {
+                    let _ = running.await;
+                }
+            });
+        }
+    });
+    addr
+}
+
+fn host(addr: SocketAddr, mac: &str, password: Option<&str>) -> Host {
+    Host {
+        name: format!("cam-{mac}"),
+        hostname: addr.ip().to_string(),
+        port: addr.port(),
+        user: "root".to_string(),
+        password: password.map(str::to_string),
+        key_generator: Some("vendor-a".to_string()),
+        device_mac: Some(mac.to_string()),
+        ..Host::default()
+    }
+}
+
+#[tokio::test]
+async fn the_generated_key_logs_in() {
+    setup();
+    let addr = serve().await;
+    let session = SshSession::connect(&host(addr, "good", None)).await;
+    assert!(session.is_ok(), "{:#}", session.err().unwrap());
+}
+
+#[tokio::test]
+async fn a_key_the_server_turns_down_falls_back_to_the_password() {
+    setup();
+    let addr = serve().await;
+    let session = SshSession::connect(&host(addr, "wrong", Some(PASSWORD))).await;
+    assert!(session.is_ok(), "{:#}", session.err().unwrap());
+}
+
+#[tokio::test]
+async fn a_generator_that_fails_falls_back_to_the_password() {
+    setup();
+    let addr = serve().await;
+    let session = SshSession::connect(&host(addr, "broken", Some(PASSWORD))).await;
+    assert!(session.is_ok(), "{:#}", session.err().unwrap());
+}
+
+#[tokio::test]
+async fn when_nothing_gets_in_the_error_says_what_the_generator_did() {
+    setup();
+    let addr = serve().await;
+    let err = SshSession::connect(&host(addr, "wrong", Some("not-it")))
+        .await
+        .err()
+        .expect("nothing should get in");
+    let message = format!("{err:#}");
+    assert!(
+        message.contains("key generator 'vendor-a': the server did not accept its key"),
+        "{message}"
+    );
+}
+
+#[tokio::test]
+async fn a_host_without_a_generator_does_not_use_one() {
+    setup();
+    let addr = serve().await;
+    // The same device, but no generator named: its key would have got in.
+    let mut plain = host(addr, "good", None);
+    plain.key_generator = None;
+    assert!(SshSession::connect(&plain).await.is_err());
+}
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-core --test key_gen_login 2>&1 | grep -E "^error" | head -3; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: 编译失败：`unresolved import omnyssh_core::ssh::key_gen`、`no variant named KeyRequired`。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r2-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r2-impl.patch && git apply /tmp/r2-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/event.rs b/crates/omnyssh-core/src/event.rs
--- a/crates/omnyssh-core/src/event.rs
+++ b/crates/omnyssh-core/src/event.rs
@@ -159,6 +159,19 @@
     /// A private key is encrypted and no passphrase is cached for it yet.
     /// Frontends prompt once per key path and call [`crate::ssh::identity::unlock`].
     KeyPassphraseRequired { host_name: HostId, key_path: String },
+    /// A login to `host_name` waits for the private key its key generator
+    /// makes: run `code`'s `generateKey({ credential, mac, revision }, address)`
+    /// (`credential` is JSON text, empty for none; no `revision` key when it is
+    /// `None`) and reply through [`crate::ssh::key_gen::answer`].
+    KeyRequired {
+        request_id: u64,
+        host_name: HostId,
+        code: String,
+        credential: String,
+        mac: String,
+        revision: Option<u32>,
+        address: String,
+    },
     /// A connection to `host_name` waits for the login password of `login`
     /// (`user@host`). Frontends answer with [`crate::ssh::password::answer`];
     /// `retry` says the previous one was refused, and `new_host_key` is the
diff --git a/crates/omnyssh-core/src/ssh/key_gen.rs b/crates/omnyssh-core/src/ssh/key_gen.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-core/src/ssh/key_gen.rs
@@ -0,0 +1,471 @@
+//! Private keys made by a key generator (`key_generators.toml`) for a host that
+//! names one: JS code the desktop frontend runs on the generator's credential and
+//! the host's MAC and revision. A login asks for the key with
+//! [`CoreEvent::KeyRequired`] and the frontend replies through [`answer`]. Keys
+//! are kept in process memory only, for as long as nothing they came from
+//! changes. Without a frontend serving requests (the TUI), nothing is asked.
+
+use std::collections::HashMap;
+use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
+use std::time::Duration;
+
+use russh::keys::key::KeyPair;
+use thiserror::Error;
+use tokio::sync::{mpsc, oneshot};
+
+use crate::config::key_generators::{load_key_generators, KeyGenerators};
+use crate::event::CoreEvent;
+use crate::ssh::client::Host;
+
+/// How long a login waits for the frontend's key. The code itself is stopped
+/// after 10 s there; this also covers a frontend that never answers.
+const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);
+
+/// What a key depends on: any change makes another.
+#[derive(Debug, Clone, PartialEq, Eq, Hash)]
+struct CacheKey {
+    generator: String,
+    code: String,
+    credential: String,
+    mac: Option<String>,
+    revision: Option<u32>,
+    host: String,
+    hostname: String,
+    user: String,
+    port: u16,
+}
+
+#[derive(Default)]
+struct State {
+    /// Where requests go: the frontend that serves them.
+    service: Option<mpsc::Sender<CoreEvent>>,
+    /// Requests waiting for an answer, by id.
+    pending: HashMap<u64, oneshot::Sender<Result<String, String>>>,
+    last_request: u64,
+    /// Generated private keys (their text), by what they were made from.
+    keys: HashMap<CacheKey, String>,
+}
+
+fn state() -> MutexGuard<'static, State> {
+    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
+    STATE
+        .get_or_init(Mutex::default)
+        .lock()
+        .unwrap_or_else(PoisonError::into_inner)
+}
+
+/// Makes `tx` the frontend that runs key generators: logins send it
+/// [`CoreEvent::KeyRequired`] and wait for [`answer`].
+pub fn serve(tx: mpsc::Sender<CoreEvent>) {
+    state().service = Some(tx);
+}
+
+/// An answer that has no request to go to.
+#[derive(Debug, Error)]
+pub enum KeyGenError {
+    /// The request was answered already, or its login stopped waiting.
+    #[error("no login is waiting for this key")]
+    NotRequested,
+}
+
+/// Answers the request `request_id` of a [`CoreEvent::KeyRequired`] with the
+/// private key text, or why there is none.
+///
+/// # Errors
+/// [`KeyGenError::NotRequested`] when no login waits on that request.
+pub fn answer(request_id: u64, key: Result<String, String>) -> Result<(), KeyGenError> {
+    let reply = state()
+        .pending
+        .remove(&request_id)
+        .ok_or(KeyGenError::NotRequested)?;
+    reply.send(key).map_err(|_| KeyGenError::NotRequested)
+}
+
+/// Reads a private key as a login would.
+///
+/// # Errors
+/// Why it cannot be used: not private key text, or protected by a passphrase.
+pub(crate) fn parse_key(text: &str) -> Result<KeyPair, String> {
+    if !text.trim_start().starts_with("-----BEGIN") {
+        return Err("the code did not return private key text".to_string());
+    }
+    russh::keys::decode_secret_key(text, None).map_err(|e| match e {
+        russh::keys::Error::KeyIsEncrypted => "the key is protected by a passphrase".to_string(),
+        e => format!("the key cannot be read: {e}"),
+    })
+}
+
+/// The `SHA256:…` fingerprint of the public half of private key `text`, read as
+/// a login reads it.
+///
+/// # Errors
+/// Why a login could not use it.
+pub fn fingerprint(text: &str) -> Result<String, String> {
+    let key = parse_key(text)?;
+    let public = key
+        .clone_public_key()
+        .map_err(|e| format!("the key cannot be read: {e}"))?;
+    Ok(format!("SHA256:{}", public.fingerprint()))
+}
+
+/// The generated key a login tries first.
+pub(crate) enum Generated {
+    /// The host has no generator, generators are off, or nothing runs them.
+    Unused,
+    Key {
+        generator: String,
+        key: Box<KeyPair>,
+    },
+    /// Nothing usable came of it; the login goes on without.
+    Failed { generator: String, reason: String },
+}
+
+/// The key `host`'s generator makes, from memory or asked of the frontend.
+pub(crate) async fn generate(host: &Host) -> Generated {
+    let Some(name) = host.key_generator.clone() else {
+        return Generated::Unused;
+    };
+    let service = state().service.clone();
+    if service.is_none() {
+        return Generated::Unused;
+    }
+    let config = match tokio::task::spawn_blocking(load_key_generators).await {
+        Ok(Ok(config)) => config,
+        Ok(Err(e)) => {
+            return Generated::Failed {
+                generator: name,
+                reason: format!("{e:#}"),
+            }
+        }
+        Err(e) => {
+            return Generated::Failed {
+                generator: name,
+                reason: e.to_string(),
+            }
+        }
+    };
+    generate_with(host, &config, service, ANSWER_TIMEOUT).await
+}
+
+async fn generate_with(
+    host: &Host,
+    config: &KeyGenerators,
+    service: Option<mpsc::Sender<CoreEvent>>,
+    wait: Duration,
+) -> Generated {
+    let (Some(name), Some(service)) = (host.key_generator.clone(), service) else {
+        return Generated::Unused;
+    };
+    if !config.enabled {
+        return Generated::Unused;
+    }
+    let failed = |reason: String| Generated::Failed {
+        generator: name.clone(),
+        reason,
+    };
+    let Some(generator) = config.find(&name) else {
+        return failed("there is no key generator by that name".to_string());
+    };
+    let cache_key = CacheKey {
+        generator: name.clone(),
+        code: generator.code.clone(),
+        credential: generator.credential.clone(),
+        mac: host.device_mac.clone(),
+        revision: host.device_revision,
+        host: host.name.clone(),
+        hostname: host.hostname.clone(),
+        user: host.user.clone(),
+        port: host.port,
+    };
+    let held = state().keys.get(&cache_key).cloned();
+    let text = match held {
+        Some(text) => text,
+        None => {
+            let event = |request_id| CoreEvent::KeyRequired {
+                request_id,
+                host_name: host.name.clone(),
+                code: generator.code.clone(),
+                credential: generator.credential.clone(),
+                mac: host.device_mac.clone().unwrap_or_default(),
+                revision: host.device_revision,
+                address: host.hostname.clone(),
+            };
+            match ask(&service, event, wait).await {
+                Ok(text) => text,
+                Err(reason) => return failed(reason),
+            }
+        }
+    };
+    match parse_key(&text) {
+        Ok(key) => {
+            state().keys.insert(cache_key, text);
+            Generated::Key {
+                generator: name,
+                key: Box::new(key),
+            }
+        }
+        Err(reason) => failed(reason),
+    }
+}
+
+/// Sends the request `event` makes of its id and waits for the answer.
+async fn ask(
+    service: &mpsc::Sender<CoreEvent>,
+    event: impl FnOnce(u64) -> CoreEvent,
+    wait: Duration,
+) -> Result<String, String> {
+    let (reply, answered) = oneshot::channel();
+    let request_id = {
+        let mut state = state();
+        state.last_request += 1;
+        let id = state.last_request;
+        state.pending.insert(id, reply);
+        id
+    };
+    let _pending = Pending(request_id);
+    if service.send(event(request_id)).await.is_err() {
+        return Err("the desktop app is not running it".to_string());
+    }
+    match tokio::time::timeout(wait, answered).await {
+        Ok(Ok(answer)) => answer,
+        Ok(Err(_)) => Err("the request was dropped".to_string()),
+        Err(_) => Err(format!("no key within {} s", wait.as_secs())),
+    }
+}
+
+/// A request waiting for its answer; unlisted once the login stops waiting,
+/// so a late answer is refused.
+struct Pending(u64);
+
+impl Drop for Pending {
+    fn drop(&mut self) {
+        state().pending.remove(&self.0);
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use std::sync::atomic::{AtomicUsize, Ordering};
+    use std::sync::Arc;
+
+    use super::*;
+    use crate::config::key_generators::KeyGenerator;
+
+    /// A fresh Ed25519 key as PKCS#8 PEM text.
+    fn key_text() -> String {
+        let key = KeyPair::generate_ed25519();
+        let mut pem = Vec::new();
+        russh::keys::encode_pkcs8_pem(&key, &mut pem).unwrap();
+        String::from_utf8(pem).unwrap()
+    }
+
+    fn config(enabled: bool) -> KeyGenerators {
+        KeyGenerators {
+            enabled,
+            generators: vec![KeyGenerator {
+                name: "vendor-a".to_string(),
+                code: "code".to_string(),
+                credential: r#"{"master_key":"k"}"#.to_string(),
+            }],
+        }
+    }
+
+    fn host(mac: &str) -> Host {
+        Host {
+            name: format!("cam-{mac}"),
+            hostname: "192.168.11.1".to_string(),
+            key_generator: Some("vendor-a".to_string()),
+            device_mac: Some(mac.to_string()),
+            device_revision: Some(2),
+            ..Host::default()
+        }
+    }
+
+    /// A frontend answering every request with `reply`, counting them and
+    /// keeping the last.
+    fn frontend(
+        reply: Result<String, String>,
+    ) -> (
+        mpsc::Sender<CoreEvent>,
+        Arc<AtomicUsize>,
+        Arc<Mutex<Option<CoreEvent>>>,
+    ) {
+        let (tx, mut rx) = mpsc::channel::<CoreEvent>(8);
+        let count = Arc::new(AtomicUsize::new(0));
+        let last = Arc::new(Mutex::new(None));
+        let (seen, kept) = (Arc::clone(&count), Arc::clone(&last));
+        tokio::spawn(async move {
+            while let Some(event) = rx.recv().await {
+                if let CoreEvent::KeyRequired { request_id, .. } = event {
+                    seen.fetch_add(1, Ordering::SeqCst);
+                    *kept.lock().unwrap() = Some(event);
+                    let _ = answer(request_id, reply.clone());
+                }
+            }
+        });
+        (tx, count, last)
+    }
+
+    const WAIT: Duration = Duration::from_secs(5);
+
+    #[tokio::test]
+    async fn the_frontend_gets_the_generator_and_the_device() {
+        let (tx, _, last) = frontend(Ok(key_text()));
+        let got = generate_with(&host("aa:01"), &config(true), Some(tx), WAIT).await;
+        assert!(matches!(got, Generated::Key { ref generator, .. } if generator == "vendor-a"));
+        let seen = last.lock().unwrap().take();
+        match seen {
+            Some(CoreEvent::KeyRequired {
+                host_name,
+                code,
+                credential,
+                mac,
+                revision,
+                address,
+                ..
+            }) => {
+                assert_eq!(host_name, "cam-aa:01");
+                assert_eq!(code, "code");
+                assert_eq!(credential, r#"{"master_key":"k"}"#);
+                assert_eq!(mac, "aa:01");
+                assert_eq!(revision, Some(2));
+                assert_eq!(address, "192.168.11.1");
+            }
+            other => panic!("expected KeyRequired, got {other:?}"),
+        }
+    }
+
+    #[tokio::test]
+    async fn a_key_is_made_once_until_what_it_comes_from_changes() {
+        let (tx, count, _) = frontend(Ok(key_text()));
+        let cfg = config(true);
+        generate_with(&host("aa:02"), &cfg, Some(tx.clone()), WAIT).await;
+        generate_with(&host("aa:02"), &cfg, Some(tx.clone()), WAIT).await;
+        assert_eq!(
+            count.load(Ordering::SeqCst),
+            1,
+            "the second login uses the held key"
+        );
+
+        let mut other = host("aa:02");
+        other.device_revision = Some(3);
+        generate_with(&other, &cfg, Some(tx.clone()), WAIT).await;
+        assert_eq!(
+            count.load(Ordering::SeqCst),
+            2,
+            "another revision is another key"
+        );
+
+        let mut changed = cfg.clone();
+        changed.generators[0].credential = r#"{"master_key":"other"}"#.to_string();
+        generate_with(&host("aa:02"), &changed, Some(tx), WAIT).await;
+        assert_eq!(
+            count.load(Ordering::SeqCst),
+            3,
+            "another credential is another key"
+        );
+    }
+
+    #[tokio::test]
+    async fn nothing_is_asked_while_switched_off_or_without_a_generator() {
+        let (tx, count, _) = frontend(Ok(key_text()));
+        let got = generate_with(&host("aa:03"), &config(false), Some(tx.clone()), WAIT).await;
+        assert!(matches!(got, Generated::Unused));
+        let mut plain = host("aa:03");
+        plain.key_generator = None;
+        let got = generate_with(&plain, &config(true), Some(tx), WAIT).await;
+        assert!(matches!(got, Generated::Unused));
+        // Without a frontend (the TUI) nothing is asked either.
+        let got = generate_with(&host("aa:03"), &config(true), None, WAIT).await;
+        assert!(matches!(got, Generated::Unused));
+        assert_eq!(count.load(Ordering::SeqCst), 0);
+    }
+
+    #[tokio::test]
+    async fn failures_say_why_and_are_not_kept() {
+        let (tx, count, _) = frontend(Err("TypeError: x is undefined".to_string()));
+        let got = generate_with(&host("aa:04"), &config(true), Some(tx.clone()), WAIT).await;
+        assert!(
+            matches!(got, Generated::Failed { ref reason, .. } if reason == "TypeError: x is undefined")
+        );
+        generate_with(&host("aa:04"), &config(true), Some(tx), WAIT).await;
+        assert_eq!(count.load(Ordering::SeqCst), 2, "a failure is asked again");
+
+        let (tx, _, _) = frontend(Ok("ssh-ed25519 AAAA".to_string()));
+        let got = generate_with(&host("aa:05"), &config(true), Some(tx), WAIT).await;
+        assert!(
+            matches!(got, Generated::Failed { ref reason, .. } if reason.contains("private key text"))
+        );
+
+        let mut unknown = host("aa:06");
+        unknown.key_generator = Some("gone".to_string());
+        let (tx, _, _) = frontend(Ok(key_text()));
+        let got = generate_with(&unknown, &config(true), Some(tx), WAIT).await;
+        assert!(
+            matches!(got, Generated::Failed { ref generator, ref reason } if generator == "gone" && reason.contains("no key generator"))
+        );
+    }
+
+    #[tokio::test]
+    async fn a_frontend_that_never_answers_is_given_up_on() {
+        let (tx, mut rx) = mpsc::channel::<CoreEvent>(8);
+        let silent = tokio::spawn(async move { while rx.recv().await.is_some() {} });
+        let got = generate_with(
+            &host("aa:07"),
+            &config(true),
+            Some(tx),
+            Duration::from_millis(50),
+        )
+        .await;
+        assert!(
+            matches!(got, Generated::Failed { ref reason, .. } if reason.starts_with("no key within"))
+        );
+        silent.abort();
+    }
+
+    /// What the vendor code returns: an OpenSSH-format Ed25519 key.
+    #[test]
+    fn an_openssh_ed25519_key_is_read() {
+        let KeyPair::Ed25519(signing) = KeyPair::generate_ed25519() else {
+            unreachable!()
+        };
+        let keypair = ssh_key::private::Ed25519Keypair {
+            public: ssh_key::public::Ed25519PublicKey(signing.verifying_key().to_bytes()),
+            private: ssh_key::private::Ed25519PrivateKey::from_bytes(&signing.to_bytes()),
+        };
+        let text = ssh_key::PrivateKey::from(keypair)
+            .to_openssh(ssh_key::LineEnding::LF)
+            .unwrap();
+        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
+        let expected = format!(
+            "SHA256:{}",
+            KeyPair::Ed25519(signing)
+                .clone_public_key()
+                .unwrap()
+                .fingerprint()
+        );
+        assert_eq!(fingerprint(&text), Ok(expected));
+    }
+
+    #[test]
+    fn a_late_answer_is_refused() {
+        assert!(matches!(
+            answer(u64::MAX, Ok(String::new())),
+            Err(KeyGenError::NotRequested)
+        ));
+    }
+
+    #[test]
+    fn fingerprints_match_the_public_key() {
+        let key = KeyPair::generate_ed25519();
+        let mut pem = Vec::new();
+        russh::keys::encode_pkcs8_pem(&key, &mut pem).unwrap();
+        let expected = format!("SHA256:{}", key.clone_public_key().unwrap().fingerprint());
+        assert_eq!(fingerprint(&String::from_utf8(pem).unwrap()), Ok(expected));
+        assert!(fingerprint("not a key").is_err());
+        assert!(fingerprint(
+            "-----BEGIN OPENSSH PRIVATE KEY-----\ngarbage\n-----END OPENSSH PRIVATE KEY-----\n"
+        )
+        .is_err());
+    }
+}
diff --git a/crates/omnyssh-core/src/ssh/mod.rs b/crates/omnyssh-core/src/ssh/mod.rs
--- a/crates/omnyssh-core/src/ssh/mod.rs
+++ b/crates/omnyssh-core/src/ssh/mod.rs
@@ -8,6 +8,7 @@
 pub mod host_key;
 pub mod identity;
 pub mod jump;
+pub mod key_gen;
 pub mod key_setup;
 mod known_hosts;
 pub mod metrics;
diff --git a/crates/omnyssh-core/src/ssh/session.rs b/crates/omnyssh-core/src/ssh/session.rs
--- a/crates/omnyssh-core/src/ssh/session.rs
+++ b/crates/omnyssh-core/src/ssh/session.rs
@@ -37,6 +37,7 @@
 use crate::ssh::client::Host;
 use crate::ssh::host_key::{self, AskHostKey, Decision, HostKeyChanged, Shown};
 use crate::ssh::identity::{self, IdentityError};
+use crate::ssh::key_gen::{self, Generated};
 use crate::ssh::known_hosts::{self, Verdict};
 use crate::ssh::password::{self, AskPassword, Method, NoAnswer, Prompt};
 
@@ -1155,7 +1156,20 @@
     F: Fn() -> Fut,
     Fut: Future<Output = anyhow::Result<Dialed>>,
 {
-    let Dialed { handle, link, .. } = first;
+    let Dialed {
+        mut handle, link, ..
+    } = first;
+    // The key a generator makes for this host goes first; whatever comes of it,
+    // the login then goes on as it always has.
+    let generated = match generated_login(&mut handle, host).await? {
+        GeneratedLogin::Accepted => return Ok(handle),
+        GeneratedLogin::Failed(note) => Some(note),
+        GeneratedLogin::Unused => None,
+    };
+    let failed = |message: String| match &generated {
+        Some(note) => format!("{message} ({note})"),
+        None => message,
+    };
     let asking = matches!(passwords, Passwords::Ask(_));
     let (handle, keys) = authenticate(handle, host, asking).await?;
     let encrypted_key = match keys {
@@ -1256,7 +1270,7 @@
                 "SSH login to {} gave up waiting for the password",
                 host.name
             ),
-            Asked::Refused => format!("SSH authentication failed for {}", host.name),
+            Asked::Refused => failed(format!("SSH authentication failed for {}", host.name)),
         };
         // No password got in: the key it skipped still may (#97).
         return Err(match locked {
@@ -1272,7 +1286,7 @@
         return Err(NoPassword::new(key, &host.name).into());
     }
 
-    let message = format!("SSH authentication failed for {}", host.name);
+    let message = failed(format!("SSH authentication failed for {}", host.name));
     // Every attempt folds a dropped link into "not accepted". A connection that
     // is gone refused us only if the server hung up itself, as OpenSSH does
     // after too many failed logins.
@@ -1477,6 +1491,48 @@
 /// How long one login request may wait for the server's verdict.
 const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
 
+/// How the key from `host`'s key generator did.
+enum GeneratedLogin {
+    /// No generator in use.
+    Unused,
+    Accepted,
+    /// Why it did not get in, for the error should nothing else either.
+    Failed(String),
+}
+
+/// Tries the key `host`'s key generator makes. Only a server that stops
+/// answering ends the login here.
+async fn generated_login(
+    handle: &mut Handle<KnownHostsHandler>,
+    host: &Host,
+) -> anyhow::Result<GeneratedLogin> {
+    let (generator, key) = match key_gen::generate(host).await {
+        Generated::Unused => return Ok(GeneratedLogin::Unused),
+        Generated::Key { generator, key } => (generator, key),
+        Generated::Failed { generator, reason } => {
+            tracing::warn!(host = %host.name, %generator, %reason, "key generator failed");
+            return Ok(GeneratedLogin::Failed(format!(
+                "key generator '{generator}': {reason}"
+            )));
+        }
+    };
+    let verdict = time::timeout(
+        AUTH_TIMEOUT,
+        handle.authenticate_publickey(host.user.clone(), Arc::new(*key)),
+    )
+    .await
+    .map_err(|_| Stalled)?;
+    let reason = match verdict {
+        Ok(true) => return Ok(GeneratedLogin::Accepted),
+        Ok(false) => "the server did not accept its key".to_string(),
+        Err(e) => e.to_string(),
+    };
+    tracing::info!(host = %host.name, %generator, %reason, "generated key not used");
+    Ok(GeneratedLogin::Failed(format!(
+        "key generator '{generator}': {reason}"
+    )))
+}
+
 enum KeyAuth {
     Accepted,
     /// No key got in. `encrypted_key` is one skipped for want of its
diff --git a/crates/omnyssh/src/app/mod.rs b/crates/omnyssh/src/app/mod.rs
--- a/crates/omnyssh/src/app/mod.rs
+++ b/crates/omnyssh/src/app/mod.rs
@@ -892,6 +892,14 @@
                     omnyssh_core::ssh::host_key::Decision::Cancel,
                 );
             }
+            // The TUI runs no key generators and never serves their requests;
+            // should one reach it anyway, the login goes on without the key.
+            CoreEvent::KeyRequired { request_id, .. } => {
+                let _ = omnyssh_core::ssh::key_gen::answer(
+                    request_id,
+                    Err("the terminal app does not run key generators".to_string()),
+                );
+            }
 
             // ----------------------------------------------------------------
             // Update checker events
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --test key_gen_login 2>&1 | grep "test result"; cargo test -p omnyssh-core --lib key_gen 2>&1 | grep "test result"; cargo clippy -p omnyssh-core -p omnyssh --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" | head -5; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: `5 passed`；`14 passed; 0 failed; 1 ignored`（被忽略的是原有的 `key_setup` 测试）；clippy 没有输出。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core crates/omnyssh/src/app/mod.rs
git commit -m "feat(core): log in with a key generator's key first"
```

---

### Task 3: GUI 后端：主机的生成器字段

**Files:** Modify `crates/omnyssh-gui/src/dto.rs`、`commands/hosts.rs`、`{UI}/src/lib/bindings.ts`

`HostDto` 带出 `keyGenerator`、`deviceMac`、`deviceRevision`；`HostInputDto.keyGen` 省略时保存和复制都保留原值（设计决定 8）。测试在 `dto.rs` 和 `hosts.rs` 里。

- [ ] **Step 1: 应用补丁**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r3-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r3-impl.patch && git apply /tmp/r3-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/src/commands/hosts.rs b/crates/omnyssh-gui/src/commands/hosts.rs
--- a/crates/omnyssh-gui/src/commands/hosts.rs
+++ b/crates/omnyssh-gui/src/commands/hosts.rs
@@ -132,6 +132,9 @@
     // Likewise an omitted upload protocol: a form that predates it must not reset
     // an SFTP host to ZMODEM.
     let upload_given = input.upload_protocol.is_some();
+    // And omitted key generator settings: a form with generators switched off
+    // leaves them out, and must not wipe them.
+    let key_gen_given = input.key_gen.is_some();
     let mut host = Host::from(input);
     match hosts.iter().position(|h| h.name == host.name) {
         Some(i) => {
@@ -143,6 +146,11 @@
             if !upload_given {
                 host.upload_protocol = existing.upload_protocol;
                 host.upload_dir = existing.upload_dir.clone();
+            }
+            if !key_gen_given {
+                host.key_generator = existing.key_generator.clone();
+                host.device_mac = existing.device_mac.clone();
+                host.device_revision = existing.device_revision;
             }
             host.password = host.password.or_else(|| existing.password.clone());
             host.identity_file = host
@@ -184,10 +192,16 @@
 fn duplicate(source: &Host, input: HostInputDto) -> Host {
     let monitoring_given = input.monitoring.is_some();
     let upload_given = input.upload_protocol.is_some();
+    let key_gen_given = input.key_gen.is_some();
     let mut host = Host::from(input);
     if !upload_given {
         host.upload_protocol = source.upload_protocol;
         host.upload_dir = source.upload_dir.clone();
+    }
+    if !key_gen_given {
+        host.key_generator = source.key_generator.clone();
+        host.device_mac = source.device_mac.clone();
+        host.device_revision = source.device_revision;
     }
     host.password = host.password.or_else(|| source.password.clone());
     host.identity_file = host.identity_file.or_else(|| source.identity_file.clone());
@@ -249,7 +263,7 @@
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::dto::{HostUploadProtocolDto, MonitorModeDto};
+    use crate::dto::{HostKeyGenDto, HostUploadProtocolDto, MonitorModeDto};
     use omnyssh_core::ssh::client::HostUploadProtocol;
     use omnyssh_core::ssh::client::{HostSource, MonitorMode};
 
@@ -271,6 +285,7 @@
             forward_agent: false,
             upload_protocol: None,
             upload_dir: None,
+            key_gen: None,
         }
     }
 
@@ -425,6 +440,29 @@
     }
 
     #[test]
+    fn upsert_keeps_key_generator_settings_the_payload_left_out() {
+        let mut hosts = vec![Host {
+            name: "cam".to_string(),
+            key_generator: Some("vendor-a".to_string()),
+            device_mac: Some("30:ae:7b:eb:17:82".to_string()),
+            device_revision: Some(2),
+            source: HostSource::Manual,
+            ..Host::default()
+        }];
+        upsert(&mut hosts, input("cam"), None);
+        assert_eq!(hosts[0].key_generator.as_deref(), Some("vendor-a"));
+        assert_eq!(hosts[0].device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
+        assert_eq!(hosts[0].device_revision, Some(2));
+
+        let mut cleared = input("cam");
+        cleared.key_gen = Some(HostKeyGenDto::default());
+        upsert(&mut hosts, cleared, None);
+        assert_eq!(hosts[0].key_generator, None);
+        assert_eq!(hosts[0].device_mac, None);
+        assert_eq!(hosts[0].device_revision, None);
+    }
+
+    #[test]
     fn upsert_applies_a_monitoring_mode_the_payload_carries() {
         let mut hosts = vec![Host {
             name: "fw".to_string(),
@@ -590,6 +628,28 @@
     }
 
     #[test]
+    fn a_duplicate_takes_the_sources_key_generator_unless_the_form_gives_its_own() {
+        let mut source = source_host();
+        source.key_generator = Some("vendor-a".to_string());
+        source.device_mac = Some("30:ae:7b:eb:17:82".to_string());
+        source.device_revision = Some(2);
+        let host = duplicate(&source, input("web-2"));
+        assert_eq!(host.key_generator.as_deref(), Some("vendor-a"));
+        assert_eq!(host.device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
+        assert_eq!(host.device_revision, Some(2));
+
+        let mut typed = input("web-3");
+        typed.key_gen = Some(HostKeyGenDto {
+            generator: Some("vendor-a".to_string()),
+            mac: Some("30:ae:7b:eb:17:83".to_string()),
+            revision: None,
+        });
+        let host = duplicate(&source, typed);
+        assert_eq!(host.device_mac.as_deref(), Some("30:ae:7b:eb:17:83"));
+        assert_eq!(host.device_revision, None);
+    }
+
+    #[test]
     fn a_duplicate_keeps_the_sources_monitoring_when_the_form_leaves_it_out() {
         let mut source = source_host();
         source.monitoring = MonitorMode::TcpPort;
diff --git a/crates/omnyssh-gui/src/dto.rs b/crates/omnyssh-gui/src/dto.rs
--- a/crates/omnyssh-gui/src/dto.rs
+++ b/crates/omnyssh-gui/src/dto.rs
@@ -143,6 +143,26 @@
     pub upload_protocol: HostUploadProtocolDto,
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub upload_dir: Option<String>,
+    /// The key generator this host's key comes from, if any.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub key_generator: Option<String>,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub device_mac: Option<String>,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub device_revision: Option<u32>,
+}
+
+/// A host's key generator settings as the form sends them. No generator clears
+/// the MAC and revision with it.
+#[derive(Debug, Clone, Default, Deserialize, specta::Type)]
+#[serde(rename_all = "camelCase")]
+pub struct HostKeyGenDto {
+    #[serde(default)]
+    pub generator: Option<String>,
+    #[serde(default)]
+    pub mac: Option<String>,
+    #[serde(default)]
+    pub revision: Option<u32>,
 }
 
 /// One `ssh -L` rule (tech-gui.md §4.1): listen on `bindAddress:bindPort` here and
@@ -207,6 +227,10 @@
     /// Blank means the home directory.
     #[serde(default)]
     pub upload_dir: Option<String>,
+    /// Left out means unchanged: the form leaves it out while key generators are
+    /// switched off, and must not wipe what was set.
+    #[serde(default)]
+    pub key_gen: Option<HostKeyGenDto>,
 }
 
 /// What this desktop allows the tray (tech-gui.md §4.2 `set_tray_behavior`): an icon
@@ -586,6 +610,9 @@
             forward_agent: host.forward_agent,
             upload_protocol: host.upload_protocol.into(),
             upload_dir: host.upload_dir.clone(),
+            key_generator: host.key_generator.clone(),
+            device_mac: host.device_mac.clone(),
+            device_revision: host.device_revision,
         }
     }
 }
@@ -653,6 +680,14 @@
         // last guard. Password is not trimmed — its bytes are preserved verbatim.
         let non_empty = |s: Option<String>| s.filter(|v| !v.is_empty());
         let monitoring: MonitorMode = dto.monitoring.map(Into::into).unwrap_or_default();
+        let trimmed = |s: Option<String>| non_empty(s.map(|v| v.trim().to_string()));
+        // No generator, no device details: they mean nothing without one.
+        let key_gen = dto.key_gen.unwrap_or_default();
+        let key_generator = trimmed(key_gen.generator);
+        let (device_mac, device_revision) = match key_generator {
+            Some(_) => (trimmed(key_gen.mac), key_gen.revision),
+            None => (None, None),
+        };
         Host {
             name: dto.name,
             hostname: dto.hostname,
@@ -678,10 +713,10 @@
             forward_agent: dto.forward_agent,
             upload_protocol: dto.upload_protocol.map(Into::into).unwrap_or_default(),
             upload_dir: non_empty(dto.upload_dir.map(|d| d.trim().to_string())),
-            // Set by `save_host` from the payload's `keyGen`, or carried over.
-            key_generator: None,
-            device_mac: None,
-            device_revision: None,
+            // `save_host` carries these over when the payload leaves `keyGen` out.
+            key_generator,
+            device_mac,
+            device_revision,
             key_setup_date: None,
             password_auth_disabled: None,
         }
@@ -1091,6 +1126,7 @@
             forward_agent: false,
             upload_protocol: None,
             upload_dir: None,
+            key_gen: None,
         }
     }
 
@@ -1115,6 +1151,38 @@
         assert!(host.original_ssh_host.is_none());
         assert!(host.key_setup_date.is_none());
         assert!(host.password_auth_disabled.is_none());
+    }
+
+    #[test]
+    fn key_generator_settings_cross_both_ways() {
+        let mut input = full_input();
+        input.key_gen = Some(HostKeyGenDto {
+            generator: Some(" vendor-a ".to_string()),
+            mac: Some(" 30:ae:7b:eb:17:82 ".to_string()),
+            revision: Some(2),
+        });
+        let host = Host::from(input);
+        assert_eq!(host.key_generator.as_deref(), Some("vendor-a"));
+        assert_eq!(host.device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
+        assert_eq!(host.device_revision, Some(2));
+        let json = serde_json::to_value(HostDto::from(&host)).unwrap();
+        assert_eq!(json["keyGenerator"], "vendor-a");
+        assert_eq!(json["deviceMac"], "30:ae:7b:eb:17:82");
+        assert_eq!(json["deviceRevision"], 2);
+
+        // No generator: the device details go with it.
+        let mut none = full_input();
+        none.key_gen = Some(HostKeyGenDto {
+            generator: Some(" ".to_string()),
+            mac: Some("30:ae:7b:eb:17:82".to_string()),
+            revision: Some(2),
+        });
+        let host = Host::from(none);
+        assert_eq!(host.key_generator, None);
+        assert_eq!(host.device_mac, None);
+        assert_eq!(host.device_revision, None);
+        let json = serde_json::to_value(HostDto::from(&host)).unwrap();
+        assert!(json.get("keyGenerator").is_none());
     }
 
     #[test]
@@ -1198,6 +1266,7 @@
             forward_agent: false,
             upload_protocol: None,
             upload_dir: Some("  ".to_string()),
+            key_gen: None,
         });
         assert!(host.identity_file.is_none());
         assert!(host.upload_dir.is_none(), "a blank upload dir means home");
diff --git a/crates/omnyssh-gui/ui/src/lib/bindings.ts b/crates/omnyssh-gui/ui/src/lib/bindings.ts
--- a/crates/omnyssh-gui/ui/src/lib/bindings.ts
+++ b/crates/omnyssh-gui/ui/src/lib/bindings.ts
@@ -804,7 +804,11 @@
  * (tech-gui.md §3.4). `hasKey` reports whether an identity file is configured;
  * the key path itself never crosses the boundary.
  */
-export type HostDto = { name: string; hostname: string; user: string; port: number; tags: string[]; notes?: string | null; source: HostSourceDto; hasKey: boolean; passwordAuthDisabled?: boolean | null; monitoring: MonitorModeDto; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean; uploadProtocol?: HostUploadProtocolDto; uploadDir?: string | null }
+export type HostDto = { name: string; hostname: string; user: string; port: number; tags: string[]; notes?: string | null; source: HostSourceDto; hasKey: boolean; passwordAuthDisabled?: boolean | null; monitoring: MonitorModeDto; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean; uploadProtocol?: HostUploadProtocolDto; uploadDir?: string | null; 
+/**
+ * The key generator this host's key comes from, if any.
+ */
+keyGenerator?: string | null; deviceMac?: string | null; deviceRevision?: number | null }
 /**
  * Inbound host form payload for `save_host` (tech-gui.md §4.1, Stage 4.1). Always
  * builds a **manual** `Host`: editing an SSH-config import saves a copy that shadows
@@ -821,7 +825,12 @@
 /**
  * Blank means the home directory.
  */
-uploadDir?: string | null }
+uploadDir?: string | null; 
+/**
+ * Left out means unchanged: the form leaves it out while key generators are
+ * switched off, and must not wipe what was set.
+ */
+keyGen?: HostKeyGenDto | null }
 /**
  * A host key no longer matches `known_hosts` on a connection the user started
  * (a terminal or files); `jumpFor` names the host a bastion was on the way to.
@@ -833,6 +842,11 @@
  * crosses: the frontend cannot hand the core a key.
  */
 export type HostKeyDecisionDto = "update" | "once" | "cancel"
+/**
+ * A host's key generator settings as the form sends them. No generator clears
+ * the MAC and revision with it.
+ */
+export type HostKeyGenDto = { generator?: string | null; mac?: string | null; revision?: number | null }
 /**
  * Host origin, mirrors `omnyssh_core::ssh::client::HostSource`.
  */
```

- [ ] **Step 2: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-gui 2>&1 | grep -E "test result|FAILED"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: `139 passed`（含 `committed_bindings_are_in_sync`）和 `10 passed`，没有 `FAILED`。

- [ ] **Step 3: Commit**

```bash
git add crates/omnyssh-gui
git commit -m "feat(gui): host key generator fields over IPC, kept when the form leaves them out"
```

---

### Task 4: GUI 后端：生成器命令和请求事件

**Files:** Create `crates/omnyssh-gui/src/commands/key_gen.rs`；Modify `commands/mod.rs`、`dto.rs`、`events.rs`、`bridge.rs`、`main.rs`、`{UI}/src/lib/bindings.ts`

命令：`list_key_generators`、`set_key_generators_enabled`、`save_key_generator(previousName, generator)`（新增/修改/改名，检查名字非空且不重复、凭证为空或合法 JSON）、`delete_key_generator`、`answer_generated_key(requestId, privateKey, error)`、`key_fingerprint`。事件 `generated-key-required`。`main.rs` 在创建 `engine_tx` 后调用 `key_gen::serve(engine_tx.clone())`。改名、删除对主机的影响用纯函数实现并测试。

- [ ] **Step 1: 应用补丁**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r4-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r4-impl.patch && git apply /tmp/r4-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/src/bridge.rs b/crates/omnyssh-gui/src/bridge.rs
--- a/crates/omnyssh-gui/src/bridge.rs
+++ b/crates/omnyssh-gui/src/bridge.rs
@@ -65,6 +65,26 @@
                 let _ = events::KeyPassphraseRequired {
                     host_name,
                     key_path,
+                }
+                .emit(&app);
+            }
+            CoreEvent::KeyRequired {
+                request_id,
+                host_name,
+                code,
+                credential,
+                mac,
+                revision,
+                address,
+            } => {
+                let _ = events::GeneratedKeyRequired {
+                    request_id,
+                    host_name,
+                    code,
+                    credential,
+                    mac,
+                    revision,
+                    address,
                 }
                 .emit(&app);
             }
diff --git a/crates/omnyssh-gui/src/commands/key_gen.rs b/crates/omnyssh-gui/src/commands/key_gen.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/src/commands/key_gen.rs
@@ -0,0 +1,277 @@
+//! Key generators (`key_generators.toml`): JS code that makes a device's SSH key
+//! from a shared credential and the host's MAC and revision. The frontend lists,
+//! edits and runs them; renaming or deleting one follows through to the hosts
+//! that name it. A login's key request arrives as `generated-key-required` and
+//! is answered with `answer_generated_key`.
+
+use omnyssh_core::config::key_generators::{
+    load_key_generators, save_key_generators, KeyGenerator, KeyGenerators,
+};
+use omnyssh_core::config::{load_hosts, save_hosts};
+use omnyssh_core::ssh::client::Host;
+use omnyssh_core::ssh::key_gen;
+
+use crate::dto::{KeyGeneratorDto, KeyGeneratorsDto};
+use crate::error::CommandError;
+
+/// The switch and every generator, credential included: the settings page
+/// edits them.
+#[tauri::command]
+#[specta::specta]
+pub async fn list_key_generators() -> Result<KeyGeneratorsDto, CommandError> {
+    let config = blocking(|| load_key_generators().map_err(|e| format!("{e:#}"))).await?;
+    Ok(KeyGeneratorsDto::from(&config))
+}
+
+/// Switch key generators on or off for every host.
+#[tauri::command]
+#[specta::specta]
+pub async fn set_key_generators_enabled(enabled: bool) -> Result<(), CommandError> {
+    update_generators(move |config| {
+        config.enabled = enabled;
+        Ok(())
+    })
+    .await
+}
+
+/// Add `generator`, or replace the one named `previous_name`. A new name moves
+/// every host that named the old one over to it. The frontend reloads the hosts
+/// afterwards when it renamed one.
+#[tauri::command]
+#[specta::specta]
+pub async fn save_key_generator(
+    previous_name: Option<String>,
+    generator: KeyGeneratorDto,
+) -> Result<(), CommandError> {
+    let generator = KeyGenerator::from(generator);
+    let renamed =
+        update_generators(move |config| upsert(config, previous_name.as_deref(), generator))
+            .await?;
+    if let Some((from, to)) = renamed {
+        update_hosts(move |hosts| rename_in(hosts, &from, &to)).await?;
+    }
+    Ok(())
+}
+
+/// Delete the generator named `name`; the hosts that named it lose it, with
+/// their MAC and revision. A missing name is a no-op. The frontend reloads the
+/// hosts afterwards.
+#[tauri::command]
+#[specta::specta]
+pub async fn delete_key_generator(name: String) -> Result<(), CommandError> {
+    let gone = name.clone();
+    update_generators(move |config| {
+        config.generators.retain(|g| g.name != gone);
+        Ok(())
+    })
+    .await?;
+    update_hosts(move |hosts| clear_in(hosts, &name)).await
+}
+
+/// Answer the `generated-key-required` request `request_id` with the private
+/// key text, or with why there is none. The key stays in memory only.
+#[tauri::command]
+#[specta::specta]
+pub fn answer_generated_key(
+    request_id: u64,
+    private_key: Option<String>,
+    error: Option<String>,
+) -> Result<(), CommandError> {
+    let answer = match private_key {
+        Some(key) => Ok(key),
+        None => Err(error.unwrap_or_else(|| "no key".to_string())),
+    };
+    key_gen::answer(request_id, answer).map_err(|e| CommandError {
+        message: e.to_string(),
+    })
+}
+
+/// The `SHA256:…` fingerprint of private key `private_key`, read as a login
+/// reads it: the settings page's test. The key is not kept.
+#[tauri::command]
+#[specta::specta]
+pub fn key_fingerprint(private_key: String) -> Result<String, CommandError> {
+    key_gen::fingerprint(&private_key).map_err(|message| CommandError { message })
+}
+
+/// Puts `generator` in place of `previous` (or adds it), checked. Returns the
+/// rename, if it was one.
+fn upsert(
+    config: &mut KeyGenerators,
+    previous: Option<&str>,
+    mut generator: KeyGenerator,
+) -> Result<Option<(String, String)>, String> {
+    generator.name = generator.name.trim().to_string();
+    if generator.name.is_empty() {
+        return Err("a key generator needs a name".to_string());
+    }
+    if !generator.credential.trim().is_empty()
+        && serde_json::from_str::<serde_json::Value>(&generator.credential).is_err()
+    {
+        return Err("the credential is not valid JSON".to_string());
+    }
+    let taken = config
+        .generators
+        .iter()
+        .any(|g| g.name == generator.name && Some(g.name.as_str()) != previous);
+    if taken {
+        return Err(format!(
+            "a key generator named '{}' already exists",
+            generator.name
+        ));
+    }
+    let renamed = previous
+        .filter(|from| *from != generator.name)
+        .map(|from| (from.to_string(), generator.name.clone()));
+    match previous.and_then(|from| config.generators.iter().position(|g| g.name == from)) {
+        Some(i) => config.generators[i] = generator,
+        None => config.generators.push(generator),
+    }
+    Ok(renamed)
+}
+
+fn rename_in(hosts: &mut [Host], from: &str, to: &str) {
+    for host in hosts {
+        if host.key_generator.as_deref() == Some(from) {
+            host.key_generator = Some(to.to_string());
+        }
+    }
+}
+
+fn clear_in(hosts: &mut [Host], name: &str) {
+    for host in hosts {
+        if host.key_generator.as_deref() == Some(name) {
+            host.key_generator = None;
+            host.device_mac = None;
+            host.device_revision = None;
+        }
+    }
+}
+
+async fn blocking<T: Send + 'static>(
+    f: impl FnOnce() -> Result<T, String> + Send + 'static,
+) -> Result<T, CommandError> {
+    tauri::async_runtime::spawn_blocking(f)
+        .await
+        .map_err(|e| CommandError {
+            message: format!("key generator task failed: {e}"),
+        })?
+        .map_err(|message| CommandError { message })
+}
+
+/// Load the generators, apply `mutate`, and write them back.
+async fn update_generators<T: Send + 'static>(
+    mutate: impl FnOnce(&mut KeyGenerators) -> Result<T, String> + Send + 'static,
+) -> Result<T, CommandError> {
+    blocking(move || {
+        let mut config = load_key_generators().map_err(|e| format!("{e:#}"))?;
+        let out = mutate(&mut config)?;
+        save_key_generators(&config).map_err(|e| format!("{e:#}"))?;
+        Ok(out)
+    })
+    .await
+}
+
+/// Load the manual hosts, apply `mutate`, and write them back.
+async fn update_hosts(
+    mutate: impl FnOnce(&mut [Host]) + Send + 'static,
+) -> Result<(), CommandError> {
+    blocking(move || {
+        let mut hosts = load_hosts().map_err(|e| e.to_string())?;
+        mutate(&mut hosts);
+        save_hosts(&hosts).map_err(|e| e.to_string())
+    })
+    .await
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn generator(name: &str) -> KeyGenerator {
+        KeyGenerator {
+            name: name.to_string(),
+            code: "code".to_string(),
+            credential: r#"{"master_key":"k"}"#.to_string(),
+        }
+    }
+
+    fn config() -> KeyGenerators {
+        KeyGenerators {
+            enabled: true,
+            generators: vec![generator("a"), generator("b")],
+        }
+    }
+
+    #[test]
+    fn a_new_generator_is_added_and_an_edit_replaces_in_place() {
+        let mut config = config();
+        assert_eq!(upsert(&mut config, None, generator(" c ")), Ok(None));
+        assert_eq!(config.generators[2].name, "c");
+
+        let mut edited = generator("a");
+        edited.code = "new".to_string();
+        assert_eq!(upsert(&mut config, Some("a"), edited), Ok(None));
+        assert_eq!(config.generators[0].code, "new");
+        assert_eq!(config.generators.len(), 3);
+    }
+
+    #[test]
+    fn a_rename_says_so_and_keeps_the_place() {
+        let mut config = config();
+        let renamed = upsert(&mut config, Some("a"), generator("z"));
+        assert_eq!(renamed, Ok(Some(("a".to_string(), "z".to_string()))));
+        assert_eq!(config.generators[0].name, "z");
+    }
+
+    #[test]
+    fn bad_generators_are_refused() {
+        let mut config = config();
+        assert_eq!(
+            upsert(&mut config, None, generator(" ")),
+            Err("a key generator needs a name".to_string())
+        );
+        assert_eq!(
+            upsert(&mut config, Some("a"), generator("b")),
+            Err("a key generator named 'b' already exists".to_string())
+        );
+        let mut bad = generator("c");
+        bad.credential = "{master_key: 1}".to_string();
+        assert_eq!(
+            upsert(&mut config, None, bad),
+            Err("the credential is not valid JSON".to_string())
+        );
+        // No credential at all is fine.
+        let mut none = generator("c");
+        none.credential = "  ".to_string();
+        assert_eq!(upsert(&mut config, None, none), Ok(None));
+    }
+
+    fn host(name: &str, generator: Option<&str>) -> Host {
+        Host {
+            name: name.to_string(),
+            key_generator: generator.map(str::to_string),
+            device_mac: generator.map(|_| "30:ae:7b:eb:17:82".to_string()),
+            device_revision: generator.map(|_| 2),
+            ..Host::default()
+        }
+    }
+
+    #[test]
+    fn renames_and_deletions_reach_the_hosts_that_name_the_generator() {
+        let mut hosts = vec![
+            host("one", Some("a")),
+            host("two", Some("b")),
+            host("three", None),
+        ];
+        rename_in(&mut hosts, "a", "z");
+        assert_eq!(hosts[0].key_generator.as_deref(), Some("z"));
+        assert_eq!(hosts[1].key_generator.as_deref(), Some("b"));
+
+        clear_in(&mut hosts, "z");
+        assert_eq!(hosts[0].key_generator, None);
+        assert_eq!(hosts[0].device_mac, None);
+        assert_eq!(hosts[0].device_revision, None);
+        assert_eq!(hosts[1].device_mac.as_deref(), Some("30:ae:7b:eb:17:82"));
+    }
+}
diff --git a/crates/omnyssh-gui/src/commands/mod.rs b/crates/omnyssh-gui/src/commands/mod.rs
--- a/crates/omnyssh-gui/src/commands/mod.rs
+++ b/crates/omnyssh-gui/src/commands/mod.rs
@@ -4,6 +4,7 @@
 pub mod auth;
 pub mod hosts;
 pub mod hotkey;
+pub mod key_gen;
 pub mod keysetup;
 pub mod quick_commands;
 pub mod serial;
diff --git a/crates/omnyssh-gui/src/dto.rs b/crates/omnyssh-gui/src/dto.rs
--- a/crates/omnyssh-gui/src/dto.rs
+++ b/crates/omnyssh-gui/src/dto.rs
@@ -5,6 +5,7 @@
 use serde::{Deserialize, Serialize};
 
 use omnyssh_core::config::app_config::UpdateConfig;
+use omnyssh_core::config::key_generators::{KeyGenerator, KeyGenerators};
 use omnyssh_core::config::quick_commands::{Ending, PayloadKind, QuickCommand, QuickGroup};
 use omnyssh_core::config::serial_devices::{LineEnding, SerialDevice};
 use omnyssh_core::config::snippets::{Snippet, SnippetScope};
@@ -921,6 +922,51 @@
     pub ending: EndingDto,
 }
 
+/// One key generator: its code (defining `generateKey(deviceInfo, address)`) and
+/// the credential (JSON text, empty for none) every device using it shares.
+#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
+#[serde(rename_all = "camelCase")]
+pub struct KeyGeneratorDto {
+    pub name: String,
+    pub code: String,
+    pub credential: String,
+}
+
+/// The key generators' switch and the generators.
+#[derive(Debug, Clone, Serialize, specta::Type)]
+#[serde(rename_all = "camelCase")]
+pub struct KeyGeneratorsDto {
+    pub enabled: bool,
+    pub generators: Vec<KeyGeneratorDto>,
+}
+
+impl From<KeyGeneratorDto> for KeyGenerator {
+    fn from(dto: KeyGeneratorDto) -> Self {
+        Self {
+            name: dto.name,
+            code: dto.code,
+            credential: dto.credential,
+        }
+    }
+}
+
+impl From<&KeyGenerators> for KeyGeneratorsDto {
+    fn from(config: &KeyGenerators) -> Self {
+        Self {
+            enabled: config.enabled,
+            generators: config
+                .generators
+                .iter()
+                .map(|g| KeyGeneratorDto {
+                    name: g.name.clone(),
+                    code: g.code.clone(),
+                    credential: g.credential.clone(),
+                })
+                .collect(),
+        }
+    }
+}
+
 #[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
 #[serde(rename_all = "camelCase")]
 pub struct QuickGroupDto {
diff --git a/crates/omnyssh-gui/src/events.rs b/crates/omnyssh-gui/src/events.rs
--- a/crates/omnyssh-gui/src/events.rs
+++ b/crates/omnyssh-gui/src/events.rs
@@ -195,6 +195,22 @@
 pub struct KeyPassphraseRequired {
     pub host_name: String,
     pub key_path: String,
+}
+
+/// A login to `hostName` waits for the key its key generator makes: run `code`'s
+/// `generateKey({ credential, mac, revision }, address)` (`credential` is JSON
+/// text, empty for none; no `revision` key when it is absent) and answer with
+/// `answer_generated_key`. Nothing waits on the user: answer at once.
+#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
+#[serde(rename_all = "camelCase")]
+pub struct GeneratedKeyRequired {
+    pub request_id: u64,
+    pub host_name: String,
+    pub code: String,
+    pub credential: String,
+    pub mac: String,
+    pub revision: Option<u32>,
+    pub address: String,
 }
 
 /// A connection waits for the login password of `login` (`user@host`). Answered
diff --git a/crates/omnyssh-gui/src/main.rs b/crates/omnyssh-gui/src/main.rs
--- a/crates/omnyssh-gui/src/main.rs
+++ b/crates/omnyssh-gui/src/main.rs
@@ -21,6 +21,10 @@
     delete_host, duplicate_host, list_hosts, refresh_metrics, reload_hosts, save_host,
 };
 use commands::hotkey::set_global_hotkey;
+use commands::key_gen::{
+    answer_generated_key, delete_key_generator, key_fingerprint, list_key_generators,
+    save_key_generator, set_key_generators_enabled,
+};
 use commands::keysetup::start_key_setup;
 use commands::quick_commands::{list_quick_commands, save_quick_commands};
 use commands::serial::{
@@ -156,6 +160,12 @@
             unlock_identity,
             answer_password,
             answer_host_key,
+            list_key_generators,
+            set_key_generators_enabled,
+            save_key_generator,
+            delete_key_generator,
+            answer_generated_key,
+            key_fingerprint,
             set_tray_behavior,
             set_global_hotkey,
             check_update,
@@ -177,6 +187,7 @@
             events::SftpOpDone,
             events::SftpDisconnected,
             events::SftpLost,
+            events::GeneratedKeyRequired,
             events::FilePreview,
             events::TransferProgress,
             events::KeySetupProgress,
@@ -405,6 +416,8 @@
             // Pre-load the shared host config so the first `list_hosts` paints
             // immediately. A load failure here is non-fatal — the frontend's
             // `reload_hosts` re-attempts and surfaces the error (tech-gui.md §3.4).
+            // The page runs key generators: logins ask it for their keys.
+            omnyssh_core::ssh::key_gen::serve(engine_tx.clone());
             let gui_state = GuiState::new(engine_tx, pty);
             if let Ok(hosts) = omnyssh_core::config::load_all_hosts() {
                 gui_state.set_hosts(hosts);
diff --git a/crates/omnyssh-gui/ui/src/lib/bindings.ts b/crates/omnyssh-gui/ui/src/lib/bindings.ts
--- a/crates/omnyssh-gui/ui/src/lib/bindings.ts
+++ b/crates/omnyssh-gui/ui/src/lib/bindings.ts
@@ -628,6 +628,79 @@
 }
 },
 /**
+ * The switch and every generator, credential included: the settings page
+ * edits them.
+ */
+async listKeyGenerators() : Promise<Result<KeyGeneratorsDto, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("list_key_generators") };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Switch key generators on or off for every host.
+ */
+async setKeyGeneratorsEnabled(enabled: boolean) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("set_key_generators_enabled", { enabled }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Add `generator`, or replace the one named `previous_name`. A new name moves
+ * every host that named the old one over to it. The frontend reloads the hosts
+ * afterwards when it renamed one.
+ */
+async saveKeyGenerator(previousName: string | null, generator: KeyGeneratorDto) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("save_key_generator", { previousName, generator }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Delete the generator named `name`; the hosts that named it lose it, with
+ * their MAC and revision. A missing name is a no-op. The frontend reloads the
+ * hosts afterwards.
+ */
+async deleteKeyGenerator(name: string) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("delete_key_generator", { name }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Answer the `generated-key-required` request `request_id` with the private
+ * key text, or with why there is none. The key stays in memory only.
+ */
+async answerGeneratedKey(requestId: number, privateKey: string | null, error: string | null) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("answer_generated_key", { requestId, privateKey, error }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * The `SHA256:…` fingerprint of private key `private_key`, read as a login
+ * reads it: the settings page's test. The key is not kept.
+ */
+async keyFingerprint(privateKey: string) : Promise<Result<string, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("key_fingerprint", { privateKey }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
  * Minimize and close to the tray, or not. Resolves to what this desktop allows —
  * a tray at all, and minimizing into it; what it does not, the window keeps doing
  * as before.
@@ -710,6 +783,7 @@
 export const events = __makeEvents__<{
 error: Error,
 filePreview: FilePreview,
+generatedKeyRequired: GeneratedKeyRequired,
 hostKeyChanged: HostKeyChanged,
 hostStatusChanged: HostStatusChanged,
 hostsLoaded: HostsLoaded,
@@ -738,6 +812,7 @@
 }>({
 error: "error",
 filePreview: "file-preview",
+generatedKeyRequired: "generated-key-required",
 hostKeyChanged: "host-key-changed",
 hostStatusChanged: "host-status-changed",
 hostsLoaded: "hosts-loaded",
@@ -800,6 +875,13 @@
  */
 export type FlowControlDto = "none" | "software" | "hardware"
 /**
+ * A login to `hostName` waits for the key its key generator makes: run `code`'s
+ * `generateKey({ credential, mac, revision }, address)` (`credential` is JSON
+ * text, empty for none; no `revision` key when it is absent) and answer with
+ * `answer_generated_key`. Nothing waits on the user: answer at once.
+ */
+export type GeneratedKeyRequired = { requestId: number; hostName: string; code: string; credential: string; mac: string; revision: number | null; address: string }
+/**
  * A host as the frontend sees it — password and private-key material omitted
  * (tech-gui.md §3.4). `hasKey` reports whether an identity file is configured;
  * the key path itself never crosses the boundary.
@@ -869,6 +951,15 @@
  * key itself.
  */
 export type KeyFingerprintDto = { keyType: string; fingerprint: string }
+/**
+ * One key generator: its code (defining `generateKey(deviceInfo, address)`) and
+ * the credential (JSON text, empty for none) every device using it shares.
+ */
+export type KeyGeneratorDto = { name: string; code: string; credential: string }
+/**
+ * The key generators' switch and the generators.
+ */
+export type KeyGeneratorsDto = { enabled: boolean; generators: KeyGeneratorDto[] }
 /**
  * A private key is encrypted and no passphrase is cached yet. Frontends prompt
  * once per key path; the passphrase never crosses back out of the backend.
```

- [ ] **Step 2: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-gui 2>&1 | grep -E "test result|FAILED"; cargo clippy -p omnyssh-gui --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" | head -5; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: `143 passed` 和 `10 passed`；clippy 没有输出。

- [ ] **Step 3: Commit**

```bash
git add crates/omnyssh-gui
git commit -m "feat(gui): key generator commands and the generated-key request"
```

---

### Task 5: 前端：在 Worker 里运行并应答登录

**Files:** Create `crates/omnyssh-gui/ui/src/lib/keyGen.ts`、`keyGen.test.ts`、`crates/omnyssh-gui/ui/e2e/key-generators.spec.ts`；Modify `ipc/commands.ts`、`ipc/subscribe.ts`、`ipc/subscribe.test.ts`

`runGenerator` 接收一个 `spawn` 参数：单元测试在进程内用假的 `self` 运行真正的 Worker 脚本（Node 有 `crypto.subtle`），真实 Worker 由 Playwright 在 Chromium 里覆盖。e2e 的桩维护生成器和主机（改名、删除会同步到主机），记录每次保存和应答；`__fire` 模拟核心发出的 `generated-key-required`。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r5-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r5-test.patch && git apply /tmp/r5-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
@@ -0,0 +1,169 @@
+import { expect, test, type Page } from '@playwright/test';
+
+// Key generators. e2e runs the static SPA with Tauri absent, so `__TAURI_INTERNALS__`
+// is stubbed at the boundary. The stub keeps the generators and the hosts the way
+// the backend would (a rename or delete follows through to the hosts), records every
+// save and answer, and fingerprints a key by its length. `__fire` plays the core's
+// `generated-key-required`; the generator then runs in a real Worker.
+const CAM = { name: 'cam', hostname: '192.168.11.1', user: 'root', port: 22, tags: [], source: 'manual', hasKey: false, localForwards: [], tunnelAutostart: false, forwardAgent: false };
+const WEB = { name: 'web', hostname: 'web.example.com', user: 'root', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false };
+
+/** Returns key text naming everything it was given. */
+const ECHO = `async function generateKey(deviceInfo, address) {
+  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(deviceInfo.mac));
+  return {
+    privateKey: '-----BEGIN OPENSSH PRIVATE KEY-----\\n' + JSON.stringify({ deviceInfo, address, bytes: digest.byteLength }) + '\\n-----END OPENSSH PRIVATE KEY-----\\n',
+    fingerprint: 'SHA256:from-the-code'
+  };
+}`;
+
+type Generators = { enabled: boolean; generators: Array<{ name: string; code: string; credential: string }> };
+type Call = { cmd: string; args: Record<string, unknown> };
+
+async function boot(page: Page, generators: Generators, hosts: unknown[] = [CAM, WEB]): Promise<void> {
+  await page.addInitScript(
+    ({ generators, hosts }) => {
+      let cbid = 0;
+      const win = window as unknown as Record<string, unknown>;
+      const listeners: Record<string, number[]> = {};
+      const calls: Call[] = [];
+      win.__calls = calls;
+      const state = { generators: structuredClone(generators), hosts: structuredClone(hosts) as Array<Record<string, unknown>> };
+      win.__state = state;
+
+      function fire(event: string, payload: unknown): void {
+        for (const id of listeners[event] ?? []) {
+          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
+          cb?.({ event, id, payload });
+        }
+      }
+      win.__fire = fire;
+
+      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
+        invoke: (cmd: string, args: Record<string, unknown>) => {
+          switch (cmd) {
+            case 'list_hosts':
+              return Promise.resolve(structuredClone(state.hosts));
+            case 'list_serial_devices':
+            case 'list_quick_commands':
+              return Promise.resolve([]);
+            case 'list_key_generators':
+              return Promise.resolve(structuredClone(state.generators));
+            case 'set_key_generators_enabled':
+              calls.push({ cmd, args });
+              state.generators.enabled = args.enabled as boolean;
+              return Promise.resolve(null);
+            case 'save_key_generator': {
+              calls.push({ cmd, args });
+              const { previousName, generator } = args as { previousName: string | null; generator: Generators['generators'][0] };
+              const i = state.generators.generators.findIndex((g) => g.name === previousName);
+              if (i >= 0) state.generators.generators[i] = generator;
+              else state.generators.generators.push(generator);
+              for (const h of state.hosts) if (previousName && h.keyGenerator === previousName) h.keyGenerator = generator.name;
+              return Promise.resolve(null);
+            }
+            case 'delete_key_generator': {
+              calls.push({ cmd, args });
+              state.generators.generators = state.generators.generators.filter((g) => g.name !== args.name);
+              for (const h of state.hosts) {
+                if (h.keyGenerator === args.name) {
+                  delete h.keyGenerator;
+                  delete h.deviceMac;
+                  delete h.deviceRevision;
+                }
+              }
+              return Promise.resolve(null);
+            }
+            case 'key_fingerprint':
+              calls.push({ cmd, args });
+              return Promise.resolve(`SHA256:len-${(args.privateKey as string).length}`);
+            case 'answer_generated_key':
+            case 'save_host':
+            case 'duplicate_host':
+              calls.push({ cmd, args });
+              return Promise.resolve(null);
+            case 'reload_hosts':
+              fire('hosts-loaded', structuredClone(state.hosts));
+              return Promise.resolve(null);
+            case 'plugin:event|listen': {
+              const { event, handler } = args as { event: string; handler: number };
+              (listeners[event] ||= []).push(handler);
+              return Promise.resolve(cbid);
+            }
+            default:
+              return Promise.resolve(null);
+          }
+        },
+        transformCallback: (cb: unknown) => {
+          const id = ++cbid;
+          win[`__cb${id}`] = cb;
+          return id;
+        },
+        unregisterCallback: (id: number) => {
+          delete win[`__cb${id}`];
+        }
+      };
+    },
+    { generators, hosts }
+  );
+  await page.goto('/');
+  await expect(page.getByText(`${hosts.length} host`)).toBeVisible();
+}
+
+const calls = (page: Page, cmd: string): Promise<Array<Record<string, unknown>>> =>
+  page.evaluate(
+    (cmd) => ((window as unknown as { __calls: Call[] }).__calls ?? []).filter((c) => c.cmd === cmd).map((c) => c.args),
+    cmd
+  );
+
+const fire = (page: Page, event: string, payload: unknown) =>
+  page.evaluate(
+    ([event, payload]) => (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire(event as string, payload),
+    [event, payload] as const
+  );
+
+const ON: Generators = {
+  enabled: true,
+  generators: [{ name: 'vendor-a', code: ECHO, credential: '{"master_key":"00ff"}' }]
+};
+
+test('a login asking for a generated key gets it from the code, run in a worker', async ({ page }) => {
+  await boot(page, ON);
+  await fire(page, 'generated-key-required', {
+    requestId: 3,
+    hostName: 'cam',
+    code: ECHO,
+    credential: '{"master_key":"00ff"}',
+    mac: '30:ae:7b:eb:17:82',
+    revision: null,
+    address: '192.168.11.1'
+  });
+  await expect.poll(() => calls(page, 'answer_generated_key')).toHaveLength(1);
+  const [answer] = await calls(page, 'answer_generated_key');
+  expect(answer.requestId).toBe(3);
+  expect(answer.error).toBeNull();
+  const echoed = JSON.parse((answer.privateKey as string).split('\n')[1]);
+  // No revision: the key is left out, for the code's own default.
+  expect(echoed).toEqual({
+    deviceInfo: { credential: { master_key: '00ff' }, mac: '30:ae:7b:eb:17:82' },
+    address: '192.168.11.1',
+    bytes: 32
+  });
+
+  // Code that fails answers with why.
+  await fire(page, 'generated-key-required', {
+    requestId: 4,
+    hostName: 'cam',
+    code: 'async function generateKey() { throw new Error("master_key is undefined"); }',
+    credential: '',
+    mac: 'x',
+    revision: 1,
+    address: '192.168.11.1'
+  });
+  await expect.poll(() => calls(page, 'answer_generated_key')).toHaveLength(2);
+  expect((await calls(page, 'answer_generated_key'))[1]).toEqual({
+    requestId: 4,
+    privateKey: null,
+    error: 'master_key is undefined'
+  });
+});
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
@@ -38,6 +38,7 @@
       updateAvailable: channel('updateAvailable'),
       keyPassphraseRequired: channel('keyPassphraseRequired'),
       passwordRequired: channel('passwordRequired'),
+      generatedKeyRequired: channel('generatedKeyRequired'),
       hostKeyChanged: channel('hostKeyChanged'),
       logStopped: channel('logStopped'),
       uploadProgress: channel('uploadProgress'),
diff --git a/crates/omnyssh-gui/ui/src/lib/keyGen.test.ts b/crates/omnyssh-gui/ui/src/lib/keyGen.test.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/keyGen.test.ts
@@ -0,0 +1,155 @@
+import { beforeEach, describe, expect, it, vi } from 'vitest';
+
+const { answered } = vi.hoisted(() => ({
+  answered: [] as Array<[number, string | null, string | null]>
+}));
+vi.mock('$lib/ipc/commands', () => ({
+  answerGeneratedKey: (id: number, key: string | null, error: string | null) => {
+    answered.push([id, key, error]);
+    return Promise.resolve();
+  }
+}));
+
+import {
+  applyGeneratedKeyRequired,
+  deviceInfo,
+  keyFromReply,
+  runGenerator,
+  workerSource,
+  type WorkerLike
+} from './keyGen';
+
+/** Runs a worker script here, against a stand-in `self`: what a Worker would do. */
+function inProcess(source: string): WorkerLike & { terminated: boolean } {
+  const worker = {
+    onmessage: null as WorkerLike['onmessage'],
+    onerror: null as WorkerLike['onerror'],
+    terminated: false,
+    postMessage(data: unknown) {
+      queueMicrotask(() => scope.onmessage?.({ data }));
+    },
+    terminate() {
+      worker.terminated = true;
+    }
+  };
+  const scope: { onmessage?: (e: { data: unknown }) => void; postMessage: (data: unknown) => void } = {
+    postMessage: (data) => {
+      if (!worker.terminated) worker.onmessage?.({ data });
+    }
+  };
+  try {
+    new Function('self', source)(scope);
+  } catch (e) {
+    queueMicrotask(() => worker.onerror?.({ message: (e as Error).message }));
+  }
+  return worker;
+}
+
+const KEY = '-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n';
+
+/** Echoes what it was given into the key text, hashing the MAC with Web Crypto. */
+const ECHO = `
+async function generateKey(deviceInfo, address) {
+  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(deviceInfo.mac));
+  const hex = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, '0')).join('');
+  return {
+    privateKey: '-----BEGIN OPENSSH PRIVATE KEY-----\\n' + JSON.stringify({ deviceInfo, address, hex }) + '\\n',
+    publicKey: 'ssh-ed25519 AAAA',
+    fingerprint: 'SHA256:abc'
+  };
+}`;
+
+const request = (code: string, over: Record<string, unknown> = {}) => ({
+  code,
+  credential: '{"master_key":"00ff"}',
+  mac: '30:ae:7b:eb:17:82',
+  revision: 2,
+  address: '192.168.11.1',
+  ...over
+});
+
+describe('deviceInfo', () => {
+  it('parses the credential and leaves revision out when there is none', () => {
+    expect(deviceInfo('{"master_key":"00ff"}', 'aa', 2)).toEqual({
+      credential: { master_key: '00ff' },
+      mac: 'aa',
+      revision: 2
+    });
+    expect(deviceInfo('', 'aa', null)).toEqual({ credential: null, mac: 'aa' });
+    expect('revision' in deviceInfo('', 'aa')).toBe(false);
+  });
+
+  it('refuses a credential that is not JSON', () => {
+    expect(() => deviceInfo('{master_key: 1}', 'aa')).toThrow('The credential is not valid JSON');
+  });
+});
+
+describe('keyFromReply', () => {
+  it('takes the private key and the reported fingerprint', () => {
+    expect(keyFromReply({ ok: true, privateKey: KEY, fingerprint: 'SHA256:x' })).toEqual({
+      privateKey: KEY,
+      fingerprint: 'SHA256:x'
+    });
+  });
+
+  it('says why there is no key', () => {
+    expect(() => keyFromReply({ ok: false, error: 'boom' })).toThrow('boom');
+    expect(() => keyFromReply({ ok: true })).toThrow('The result has no privateKey');
+    expect(() => keyFromReply({ ok: true, privateKey: 'ssh-ed25519 AAAA' })).toThrow('not private key text');
+  });
+});
+
+describe('runGenerator', () => {
+  it('calls generateKey with the device info and the address, Web Crypto included', async () => {
+    const key = await runGenerator(request(ECHO), { spawn: inProcess });
+    expect(key.fingerprint).toBe('SHA256:abc');
+    const echoed = JSON.parse(key.privateKey.split('\n')[1]);
+    expect(echoed.deviceInfo).toEqual({ credential: { master_key: '00ff' }, mac: '30:ae:7b:eb:17:82', revision: 2 });
+    expect(echoed.address).toBe('192.168.11.1');
+    expect(echoed.hex).toMatch(/^[0-9a-f]{64}$/);
+  });
+
+  it('reports what the code threw, and code without generateKey', async () => {
+    const thrown = 'async function generateKey() { throw new Error("master_key is undefined"); }';
+    await expect(runGenerator(request(thrown), { spawn: inProcess })).rejects.toThrow('master_key is undefined');
+    await expect(runGenerator(request('const x = 1;'), { spawn: inProcess })).rejects.toThrow(
+      'The code does not define generateKey'
+    );
+    await expect(runGenerator(request('function ('), { spawn: inProcess })).rejects.toThrow();
+  });
+
+  it('refuses a credential that is not JSON before running anything', async () => {
+    const spawn = vi.fn(inProcess);
+    await expect(runGenerator(request(ECHO, { credential: 'nope' }), { spawn })).rejects.toThrow('not valid JSON');
+    expect(spawn).not.toHaveBeenCalled();
+  });
+
+  it('stops code that runs too long, and every worker when it is done', async () => {
+    const workers: Array<{ terminated: boolean }> = [];
+    const spawn = (source: string) => {
+      const w = inProcess(source);
+      workers.push(w);
+      return w;
+    };
+    const hangs = 'async function generateKey() { await new Promise(() => {}); }';
+    await expect(runGenerator(request(hangs), { spawn, timeoutMs: 50 })).rejects.toThrow('did not finish within');
+    await runGenerator(request(ECHO), { spawn });
+    expect(workers.map((w) => w.terminated)).toEqual([true, true]);
+  });
+});
+
+describe('applyGeneratedKeyRequired', () => {
+  beforeEach(() => answered.splice(0));
+
+  const payload = { requestId: 7, hostName: 'cam', ...request(ECHO) };
+
+  it('answers with the key', async () => {
+    await applyGeneratedKeyRequired(payload, () => Promise.resolve({ privateKey: KEY }));
+    expect(answered).toEqual([[7, KEY, null]]);
+  });
+
+  it('answers with why there is none', async () => {
+    await applyGeneratedKeyRequired(payload, () => Promise.reject(new Error('boom')));
+    expect(answered).toEqual([[7, null, 'boom']]);
+  });
+});
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/keyGen.test.ts src/lib/ipc/subscribe.test.ts 2>&1 | grep -E "Tests |Error:" | head -3; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `Failed to load url ./keyGen`（`subscribe.test.ts` 的 7 个照常通过）；e2e 1 个失败：`expect(received).toHaveLength(expected)`（没有应答）。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r5-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r5-impl.patch && git apply /tmp/r5-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts b/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
@@ -9,6 +9,8 @@
   HostInputDto,
   HostUploadProtocolDto,
   HostKeyDecisionDto,
+  KeyGeneratorDto,
+  KeyGeneratorsDto,
   LogModeDto,
   LogStatusDto,
   QuickGroupDto,
@@ -346,6 +348,52 @@
   if (res.status === 'error') throw new Error(res.error.message);
 }
 
+/** The key generators' switch and the generators, credentials included. */
+export async function listKeyGenerators(): Promise<KeyGeneratorsDto> {
+  const res = await commands.listKeyGenerators();
+  if (res.status === 'error') throw new Error(res.error.message);
+  return res.data;
+}
+
+/** Switch key generators on or off for every host. */
+export async function setKeyGeneratorsEnabled(enabled: boolean): Promise<void> {
+  const res = await commands.setKeyGeneratorsEnabled(enabled);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** Add a generator, or replace the one named `previousName` (renaming its hosts'
+ *  references). Reload the hosts after a rename. */
+export async function saveKeyGenerator(
+  previousName: string | null,
+  generator: KeyGeneratorDto
+): Promise<void> {
+  const res = await commands.saveKeyGenerator(previousName, generator);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** Delete a generator; the hosts naming it lose it. Reload the hosts after. */
+export async function deleteKeyGenerator(name: string): Promise<void> {
+  const res = await commands.deleteKeyGenerator(name);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** Answer a `generated-key-required` request: the private key, or why there is none. */
+export async function answerGeneratedKey(
+  requestId: number,
+  privateKey: string | null,
+  error: string | null
+): Promise<void> {
+  const res = await commands.answerGeneratedKey(requestId, privateKey, error);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** The `SHA256:…` fingerprint of a private key, read as a login reads it. */
+export async function keyFingerprint(privateKey: string): Promise<string> {
+  const res = await commands.keyFingerprint(privateKey);
+  if (res.status === 'error') throw new Error(res.error.message);
+  return res.data;
+}
+
 /** Answer a changed host key: save it and connect, connect once, or give up. */
 export async function answerHostKey(requestId: number, decision: HostKeyDecisionDto): Promise<void> {
   const res = await commands.answerHostKey(requestId, decision);
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
@@ -31,6 +31,7 @@
   applyHostKeyChanged
 } from './router';
 import { applyLogStopped } from '$lib/stores/sessionLogs';
+import { applyGeneratedKeyRequired } from '$lib/keyGen';
 import { applyUploadFinished, applyUploadProgress } from '$lib/stores/uploads';
 
 export async function startEventBridge(): Promise<() => void> {
@@ -58,6 +59,9 @@
     offs.push(await events.updateAvailable.listen((e) => applyUpdateAvailable(e.payload)));
     offs.push(await events.keyPassphraseRequired.listen((e) => applyKeyPassphraseRequired(e.payload)));
     offs.push(await events.passwordRequired.listen((e) => applyPasswordRequired(e.payload)));
+    offs.push(
+      await events.generatedKeyRequired.listen((e) => void applyGeneratedKeyRequired(e.payload))
+    );
     offs.push(await events.hostKeyChanged.listen((e) => applyHostKeyChanged(e.payload)));
     offs.push(await events.logStopped.listen((e) => applyLogStopped(e.payload)));
     offs.push(await events.uploadProgress.listen((e) => applyUploadProgress(e.payload)));
diff --git a/crates/omnyssh-gui/ui/src/lib/keyGen.ts b/crates/omnyssh-gui/ui/src/lib/keyGen.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/keyGen.ts
@@ -0,0 +1,151 @@
+// Key generators: user code defining `generateKey(deviceInfo, address)`, which
+// returns `{ privateKey, fingerprint, … }` for one device. It runs in a Web Worker
+// of its own — `crypto.subtle` and plain JS, none of the app — made fresh for each
+// run and stopped after RUN_TIMEOUT_MS. Logins ask for keys with
+// `generated-key-required`; the settings page runs a generator to test it.
+import type { GeneratedKeyRequired } from '$lib/bindings';
+import { answerGeneratedKey } from '$lib/ipc/commands';
+
+/** How long one run may take before its worker is stopped. */
+export const RUN_TIMEOUT_MS = 10_000;
+
+export interface KeyRequest {
+  code: string;
+  /** JSON text; empty for none. */
+  credential: string;
+  mac: string;
+  /** Left out of deviceInfo when absent: the code applies its own default. */
+  revision?: number | null;
+  /** The host's address, `generateKey`'s second argument. */
+  address: string;
+}
+
+/** What the app takes from `generateKey`'s result. */
+export interface GeneratedKey {
+  privateKey: string;
+  /** As the code reports it, to compare with the key's real one. */
+  fingerprint?: string;
+}
+
+/** `generateKey`'s first argument: `{ credential, mac, revision? }`. */
+export function deviceInfo(
+  credential: string,
+  mac: string,
+  revision?: number | null
+): Record<string, unknown> {
+  let parsed: unknown = null;
+  if (credential.trim() !== '') {
+    try {
+      parsed = JSON.parse(credential);
+    } catch {
+      throw new Error('The credential is not valid JSON');
+    }
+  }
+  const info: Record<string, unknown> = { credential: parsed, mac };
+  if (revision != null) info.revision = revision;
+  return info;
+}
+
+/** The worker's script: the user's code, then a handler that calls it once. */
+export function workerSource(code: string): string {
+  return `${code}
+;self.onmessage = async (e) => {
+  try {
+    if (typeof generateKey !== 'function') throw new Error('The code does not define generateKey');
+    const keys = await generateKey(e.data.deviceInfo, e.data.address);
+    self.postMessage({ ok: true, privateKey: keys && keys.privateKey, fingerprint: keys && keys.fingerprint });
+  } catch (err) {
+    self.postMessage({ ok: false, error: err instanceof Error ? err.message : String(err) });
+  }
+};
+`;
+}
+
+/** The key in a worker's reply, or why there is none. */
+export function keyFromReply(reply: unknown): GeneratedKey {
+  const r = (reply ?? {}) as { ok?: boolean; error?: unknown; privateKey?: unknown; fingerprint?: unknown };
+  if (!r.ok) throw new Error(typeof r.error === 'string' ? r.error : 'The code failed');
+  if (typeof r.privateKey !== 'string') throw new Error('The result has no privateKey');
+  if (!r.privateKey.trimStart().startsWith('-----BEGIN')) {
+    throw new Error('privateKey is not private key text (-----BEGIN …)');
+  }
+  return {
+    privateKey: r.privateKey,
+    fingerprint: typeof r.fingerprint === 'string' ? r.fingerprint : undefined
+  };
+}
+
+/** The part of a Worker the runner uses. */
+export interface WorkerLike {
+  onmessage: ((e: { data: unknown }) => void) | null;
+  onerror: ((e: { message?: string }) => void) | null;
+  postMessage(message: unknown): void;
+  terminate(): void;
+}
+
+/** A real Worker running `source`. */
+export function spawnWorker(source: string): WorkerLike {
+  const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
+  const worker = new Worker(url);
+  const terminate = worker.terminate.bind(worker);
+  // The URL must outlive the worker's load; let it go with the worker.
+  worker.terminate = () => {
+    terminate();
+    URL.revokeObjectURL(url);
+  };
+  return worker as unknown as WorkerLike;
+}
+
+/** Runs `request`'s generator once. Rejects with the reason it gave no key. */
+export function runGenerator(
+  request: KeyRequest,
+  { spawn = spawnWorker, timeoutMs = RUN_TIMEOUT_MS }: { spawn?: (source: string) => WorkerLike; timeoutMs?: number } = {}
+): Promise<GeneratedKey> {
+  let info: Record<string, unknown>;
+  try {
+    info = deviceInfo(request.credential, request.mac, request.revision);
+  } catch (e) {
+    return Promise.reject(e);
+  }
+  return new Promise((resolve, reject) => {
+    const worker = spawn(workerSource(request.code));
+    const finish = (settle: () => void): void => {
+      clearTimeout(timer);
+      worker.onmessage = null;
+      worker.onerror = null;
+      worker.terminate();
+      settle();
+    };
+    const timer = setTimeout(
+      () => finish(() => reject(new Error(`The code did not finish within ${timeoutMs / 1000} s`))),
+      timeoutMs
+    );
+    worker.onmessage = (e) =>
+      finish(() => {
+        try {
+          resolve(keyFromReply(e.data));
+        } catch (err) {
+          reject(err);
+        }
+      });
+    // A syntax error in the code, or one thrown before generateKey is reached.
+    worker.onerror = (e) => finish(() => reject(new Error(e.message || 'The code failed to load')));
+    worker.postMessage({ deviceInfo: info, address: request.address });
+  });
+}
+
+/** Answers a login's key request: the key, or why there is none. */
+export async function applyGeneratedKeyRequired(
+  payload: GeneratedKeyRequired,
+  run: (request: KeyRequest) => Promise<GeneratedKey> = runGenerator
+): Promise<void> {
+  let key: string | null = null;
+  let error: string | null = null;
+  try {
+    key = (await run(payload)).privateKey;
+  } catch (e) {
+    error = e instanceof Error ? e.message : String(e);
+  }
+  // The login may have stopped waiting; nothing else is waiting on this.
+  await answerGeneratedKey(payload.requestId, key, error).catch(() => {});
+}
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npm run check 2>&1 | tail -1; npx vitest run src/lib/keyGen.test.ts src/lib/ipc/subscribe.test.ts 2>&1 | grep "Tests "; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: svelte-check `0 ERRORS 0 WARNINGS`；`17 passed`；e2e `1 passed`。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
git commit -m "feat(ui): run key generators in a worker and answer logins"
```

---

### Task 6: 前端：设置页

**Files:** Create `crates/omnyssh-gui/ui/src/lib/stores/keyGenerators.ts`、`screens/keyGenForm.ts`、`keyGenForm.test.ts`、`KeyGeneratorEditor.svelte`、`KeyGeneratorSettings.svelte`；Modify `screens/Settings.svelte`、`e2e/key-generators.spec.ts`

设置页在"Terminal"之后新增"Key generators"一节。e2e 桩补上设置页打开时会调用的 `set_tray_behavior`、`load_update_config`。新增 3 个 e2e 用例：开关默认关、打开后新增生成器（凭证不是 JSON 时不能保存）；测试区显示指纹、指纹不一致提示、返回值缺 `privateKey` 时的错误；改名和删除同步到主机、删除时提示哪些主机在用。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r6-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r6-test.patch && git apply /tmp/r6-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
--- a/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
+++ b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
@@ -47,6 +47,11 @@
             case 'list_serial_devices':
             case 'list_quick_commands':
               return Promise.resolve([]);
+            // The settings page asks for these as it opens.
+            case 'set_tray_behavior':
+              return Promise.resolve({ available: true, minimize: true });
+            case 'load_update_config':
+              return Promise.resolve({ checkOnStartup: false, skipVersion: '' });
             case 'list_key_generators':
               return Promise.resolve(structuredClone(state.generators));
             case 'set_key_generators_enabled':
@@ -167,3 +172,78 @@
     error: 'master_key is undefined'
   });
 });
+
+async function openSettings(page: Page): Promise<void> {
+  await page.getByRole('button', { name: 'Settings' }).click();
+  await expect(page.getByRole('heading', { name: 'Key generators' })).toBeVisible();
+}
+
+test('the switch is off at first and shows nothing else; on, generators can be added', async ({ page }) => {
+  await boot(page, { enabled: false, generators: [] });
+  await openSettings(page);
+  const toggle = page.getByRole('switch', { name: 'Use key generators' });
+  await expect(toggle).toHaveAttribute('aria-checked', 'false');
+  await expect(page.getByRole('button', { name: 'Add key generator' })).toHaveCount(0);
+
+  await toggle.click();
+  await expect(toggle).toHaveAttribute('aria-checked', 'true');
+  expect(await calls(page, 'set_key_generators_enabled')).toEqual([{ enabled: true }]);
+
+  await page.getByRole('button', { name: 'Add key generator' }).click();
+  const editor = page.getByRole('dialog', { name: 'Add key generator' });
+  await editor.getByLabel('Name').fill('vendor-a');
+  await editor.getByLabel('Code').fill(ECHO);
+  // A credential that is not JSON is not saved.
+  await editor.getByLabel('Credential (JSON)').fill('{master_key: 1}');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect(editor.getByRole('alert')).toHaveText('The credential is not valid JSON');
+  await editor.getByLabel('Credential (JSON)').fill('{"master_key":"00ff"}');
+  await editor.getByRole('button', { name: 'Save' }).click();
+
+  await expect(page.getByRole('dialog')).toHaveCount(0);
+  expect(await calls(page, 'save_key_generator')).toEqual([
+    { previousName: null, generator: { name: 'vendor-a', code: ECHO, credential: '{"master_key":"00ff"}' } }
+  ]);
+  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('vendor-a');
+});
+
+test('the test panel runs the code and shows the key fingerprint, or why there is none', async ({ page }) => {
+  await boot(page, ON);
+  await openSettings(page);
+  await page.getByRole('button', { name: 'Edit vendor-a' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit key generator' });
+  await editor.getByLabel('Test MAC').fill('30:ae:7b:eb:17:82');
+  await editor.getByLabel('Test address').fill('192.168.11.1');
+  await editor.getByRole('button', { name: 'Run' }).click();
+  await expect(editor.getByTestId('test-fingerprint')).toHaveText(/^SHA256:len-\d+$/);
+  // The stub's fingerprint is not the one the code reports.
+  await expect(editor.getByText("The fingerprint the code returned is not this key's.")).toBeVisible();
+  const [fingerprinted] = await calls(page, 'key_fingerprint');
+  expect(fingerprinted.privateKey as string).toContain('"mac":"30:ae:7b:eb:17:82"');
+
+  await editor.getByLabel('Code').fill('async function generateKey() { return { publicKey: "x" }; }');
+  await editor.getByRole('button', { name: 'Run' }).click();
+  await expect(editor.getByTestId('test-error')).toHaveText('The result has no privateKey');
+});
+
+test('a rename and a delete reach the hosts that use the generator', async ({ page }) => {
+  await boot(page, ON, [{ ...CAM, keyGenerator: 'vendor-a', deviceMac: '30:ae:7b:eb:17:82' }, WEB]);
+  await openSettings(page);
+  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('1 host');
+
+  await page.getByRole('button', { name: 'Edit vendor-a' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit key generator' });
+  await editor.getByLabel('Name').fill('vendor-b');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect(page.getByRole('dialog')).toHaveCount(0);
+  expect((await calls(page, 'save_key_generator'))[0].previousName).toBe('vendor-a');
+  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('vendor-b');
+  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('1 host');
+
+  await page.getByRole('button', { name: 'Delete vendor-b' }).click();
+  const confirm = page.getByRole('dialog', { name: 'Delete key generator' });
+  await expect(confirm).toContainText('cam uses it');
+  await confirm.getByRole('button', { name: 'Delete' }).click();
+  expect(await calls(page, 'delete_key_generator')).toEqual([{ name: 'vendor-b' }]);
+  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('No key generators yet.');
+});
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.test.ts b/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.test.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.test.ts
@@ -0,0 +1,53 @@
+import { describe, expect, it } from 'vitest';
+import type { HostDto } from '$lib/bindings';
+import { emptyGeneratorFields, fingerprintsAgree, generatorFromFields, hostsUsing } from './keyGenForm';
+
+describe('generatorFromFields', () => {
+  const fields = { name: ' vendor-a ', code: 'code', credential: ' {"master_key":"00ff"} ' };
+
+  it('trims the name and the credential', () => {
+    expect(generatorFromFields(fields, [])).toEqual({
+      ok: true,
+      generator: { name: 'vendor-a', code: 'code', credential: '{"master_key":"00ff"}' }
+    });
+  });
+
+  it('wants a free name and a JSON credential, or none', () => {
+    expect(generatorFromFields({ ...fields, name: ' ' }, [])).toEqual({ ok: false, error: 'Name is required' });
+    expect(generatorFromFields(fields, ['vendor-a'])).toEqual({
+      ok: false,
+      error: 'A key generator named "vendor-a" already exists'
+    });
+    expect(generatorFromFields({ ...fields, credential: '{master_key: 1}' }, [])).toEqual({
+      ok: false,
+      error: 'The credential is not valid JSON'
+    });
+    expect(generatorFromFields({ ...fields, credential: '' }, []).ok).toBe(true);
+  });
+
+  it('starts a new generator from a template that defines generateKey', () => {
+    expect(emptyGeneratorFields().code).toContain('async function generateKey(deviceInfo, address)');
+  });
+});
+
+describe('fingerprintsAgree', () => {
+  it('compares with or without the SHA256: prefix and base64 padding', () => {
+    expect(fingerprintsAgree('SHA256:abc', 'SHA256:abc')).toBe(true);
+    expect(fingerprintsAgree('SHA256:abc', 'abc=')).toBe(true);
+    expect(fingerprintsAgree('SHA256:abc', 'SHA256:abd')).toBe(false);
+    // Code that reports none has nothing to disagree with.
+    expect(fingerprintsAgree('SHA256:abc', undefined)).toBe(true);
+  });
+});
+
+describe('hostsUsing', () => {
+  it('names the hosts that use a generator', () => {
+    const hosts = [
+      { name: 'cam-1', keyGenerator: 'vendor-a' },
+      { name: 'web' },
+      { name: 'cam-2', keyGenerator: 'vendor-a' }
+    ] as HostDto[];
+    expect(hostsUsing(hosts, 'vendor-a')).toEqual(['cam-1', 'cam-2']);
+    expect(hostsUsing(hosts, 'other')).toEqual([]);
+  });
+});
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/screens/keyGenForm.test.ts 2>&1 | grep -E "Tests |Error:" | head -3; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `Failed to load url ./keyGenForm`；e2e 3 个失败（`getByRole('heading', { name: 'Key generators' })` 找不到），原有 1 个通过。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r6-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r6-impl.patch && git apply /tmp/r6-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorEditor.svelte b/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorEditor.svelte
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorEditor.svelte
@@ -0,0 +1,150 @@
+<script lang="ts">
+  // Add or edit a key generator: its name, its code and its credential, and a test
+  // panel that runs the code as a login would and shows the key's fingerprint, read
+  // by the backend as a login reads it. The private key itself is never shown.
+  import type { KeyGeneratorDto } from '$lib/bindings';
+  import { Button } from '$lib/theme';
+  import Modal from '$lib/components/Modal.svelte';
+  import { keyFingerprint } from '$lib/ipc/commands';
+  import { runGenerator } from '$lib/keyGen';
+  import { fingerprintsAgree, generatorFromFields, type GeneratorFields } from './keyGenForm';
+
+  let {
+    mode,
+    initial,
+    taken,
+    onSubmit,
+    onCancel
+  }: {
+    mode: 'add' | 'edit';
+    initial: GeneratorFields;
+    /** The other generators' names. */
+    taken: readonly string[];
+    onSubmit: (generator: KeyGeneratorDto) => Promise<void>;
+    onCancel: () => void;
+  } = $props();
+
+  // Seeded once; the editor is remounted per open.
+  // svelte-ignore state_referenced_locally
+  let fields = $state<GeneratorFields>({ ...initial });
+  let error = $state<string | null>(null);
+  let saving = $state(false);
+
+  let testMac = $state('');
+  let testRevision = $state('');
+  let testAddress = $state('');
+  type Outcome = { kind: 'ok'; fingerprint: string; agrees: boolean } | { kind: 'error'; message: string };
+  let outcome = $state<Outcome | null>(null);
+  let running = $state(false);
+
+  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
+
+  async function runTest(): Promise<void> {
+    const revision = testRevision.trim();
+    if (revision !== '' && !/^\d+$/.test(revision)) {
+      outcome = { kind: 'error', message: 'Revision must be a whole number' };
+      return;
+    }
+    running = true;
+    outcome = null;
+    try {
+      const key = await runGenerator({
+        code: fields.code,
+        credential: fields.credential,
+        mac: testMac.trim(),
+        revision: revision === '' ? null : Number(revision),
+        address: testAddress.trim()
+      });
+      const fingerprint = await keyFingerprint(key.privateKey);
+      outcome = { kind: 'ok', fingerprint, agrees: fingerprintsAgree(fingerprint, key.fingerprint) };
+    } catch (e) {
+      outcome = { kind: 'error', message: message(e) };
+    } finally {
+      running = false;
+    }
+  }
+
+  async function save(): Promise<void> {
+    const result = generatorFromFields(fields, taken);
+    if (!result.ok) {
+      error = result.error;
+      return;
+    }
+    error = null;
+    saving = true;
+    try {
+      await onSubmit(result.generator);
+    } catch (e) {
+      error = message(e);
+    } finally {
+      saving = false;
+    }
+  }
+
+  const label = 'block space-y-1 text-xs font-medium text-muted';
+  const field =
+    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
+    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint';
+  const mono = `${field} resize-y font-mono text-xs`;
+</script>
+
+<Modal label={mode === 'add' ? 'Add key generator' : 'Edit key generator'} onClose={onCancel}>
+  <form
+    class="flex max-h-[76vh] w-[40rem] max-w-full flex-col"
+    onsubmit={(e) => {
+      e.preventDefault();
+      void save();
+    }}
+  >
+    <header class="border-b border-default px-5 py-3.5">
+      <h2 class="text-sm font-semibold">{mode === 'add' ? 'Add key generator' : 'Edit key generator'}</h2>
+    </header>
+    <div class="min-h-0 flex-1 space-y-3.5 overflow-y-auto px-5 py-4">
+      <label class={label}>
+        <span>Name</span>
+        <!-- svelte-ignore a11y_autofocus -->
+        <input bind:value={fields.name} class={field} autocomplete="off" autofocus />
+      </label>
+      <label class={label}>
+        <span>Code</span>
+        <textarea bind:value={fields.code} rows="12" class={mono} spellcheck="false"></textarea>
+      </label>
+      <p class="text-xs text-faint">
+        Defines <span class="font-mono">generateKey(deviceInfo, address)</span>, called with
+        <span class="font-mono">{'{ credential, mac, revision }'}</span> and the host's address, returning
+        <span class="font-mono">{'{ privateKey, … }'}</span>. Runs in a worker of its own: Web Crypto and plain JS.
+      </p>
+      <label class={label}>
+        <span>Credential (JSON)</span>
+        <textarea bind:value={fields.credential} rows="4" class={mono} spellcheck="false" placeholder={'{ "master_key": "…" }'}
+        ></textarea>
+      </label>
+
+      <fieldset class="space-y-2 rounded-lg border border-default p-3">
+        <legend class="px-1 text-xs font-medium text-muted">Test</legend>
+        <div class="grid grid-cols-[1fr,6rem,1fr] gap-2">
+          <input bind:value={testMac} class={field} placeholder="MAC" aria-label="Test MAC" autocomplete="off" />
+          <input bind:value={testRevision} class={field} placeholder="Revision" aria-label="Test revision" inputmode="numeric" />
+          <input bind:value={testAddress} class={field} placeholder="Address" aria-label="Test address" autocomplete="off" />
+        </div>
+        <div class="flex items-center gap-3">
+          <Button variant="ghost" onclick={runTest} disabled={running}>{running ? 'Running…' : 'Run'}</Button>
+          {#if outcome?.kind === 'ok'}
+            <span class="min-w-0 truncate font-mono text-xs text-fg" data-testid="test-fingerprint">{outcome.fingerprint}</span>
+          {:else if outcome?.kind === 'error'}
+            <span class="min-w-0 text-xs text-status-crit" data-testid="test-error">{outcome.message}</span>
+          {/if}
+        </div>
+        {#if outcome?.kind === 'ok' && !outcome.agrees}
+          <p class="text-xs text-status-warn">The fingerprint the code returned is not this key's.</p>
+        {/if}
+      </fieldset>
+
+      {#if error}<p class="text-sm text-status-crit" role="alert">{error}</p>{/if}
+    </div>
+    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
+      <Button variant="ghost" onclick={onCancel}>Cancel</Button>
+      <Button variant="primary" type="submit" disabled={saving}>Save</Button>
+    </footer>
+  </form>
+</Modal>
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorSettings.svelte b/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorSettings.svelte
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/KeyGeneratorSettings.svelte
@@ -0,0 +1,144 @@
+<script lang="ts">
+  // Settings › Key generators: the switch (off by default; off, nothing changes
+  // anywhere) and, on, the generators with add / edit / delete. Renaming or deleting
+  // one follows through to the hosts that name it, so the hosts are reloaded after.
+  import { onMount } from 'svelte';
+  import type { KeyGeneratorDto } from '$lib/bindings';
+  import { Button, Icon } from '$lib/theme';
+  import Modal from '$lib/components/Modal.svelte';
+  import { deleteKeyGenerator, reloadHosts, saveKeyGenerator, setKeyGeneratorsEnabled } from '$lib/ipc/commands';
+  import { hosts } from '$lib/stores/hosts';
+  import { keyGenerators, loadKeyGenerators } from '$lib/stores/keyGenerators';
+  import { lastError } from '$lib/stores/notifications';
+  import KeyGeneratorEditor from './KeyGeneratorEditor.svelte';
+  import { emptyGeneratorFields, hostsUsing } from './keyGenForm';
+
+  onMount(() => void loadKeyGenerators());
+
+  type Dialog = { kind: 'add' } | { kind: 'edit'; generator: KeyGeneratorDto } | { kind: 'delete'; name: string };
+  let dialog = $state<Dialog | null>(null);
+
+  const enabled = $derived($keyGenerators?.enabled ?? false);
+  const generators = $derived($keyGenerators?.generators ?? []);
+  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
+
+  async function toggle(): Promise<void> {
+    try {
+      await setKeyGeneratorsEnabled(!enabled);
+    } catch (e) {
+      lastError.set(`Key generators: ${message(e)}`);
+    }
+    await loadKeyGenerators();
+  }
+
+  async function save(previous: string | null, generator: KeyGeneratorDto): Promise<void> {
+    await saveKeyGenerator(previous, generator);
+    await loadKeyGenerators();
+    if (previous !== null && previous !== generator.name) await reloadHosts();
+    dialog = null;
+  }
+
+  async function remove(name: string): Promise<void> {
+    try {
+      await deleteKeyGenerator(name);
+      await loadKeyGenerators();
+      await reloadHosts();
+    } catch (e) {
+      lastError.set(`Delete key generator: ${message(e)}`);
+    }
+    dialog = null;
+  }
+
+  const iconBtn =
+    'grid h-7 w-7 place-items-center rounded-lg text-muted transition hover:bg-surface-inset hover:text-fg ' +
+    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
+</script>
+
+<div class="space-y-3">
+  <div class="flex items-center justify-between gap-4">
+    <div class="min-w-0">
+      <p class="text-sm">Use key generators</p>
+      <p class="text-xs text-muted">
+        Hosts set up with one try the key it makes first, then log in as usual.
+      </p>
+    </div>
+    <button
+      type="button"
+      role="switch"
+      aria-checked={enabled}
+      aria-label="Use key generators"
+      onclick={toggle}
+      class="relative h-6 w-11 shrink-0 rounded-full transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {enabled
+        ? 'bg-accent'
+        : 'bg-surface-inset'}"
+    >
+      <span
+        class="absolute top-0.5 h-5 w-5 rounded-full bg-surface shadow-soft transition-[left] {enabled
+          ? 'left-[1.375rem]'
+          : 'left-0.5'}"
+      ></span>
+    </button>
+  </div>
+
+  {#if enabled}
+    <ul class="divide-y divide-default rounded-lg border border-default" aria-label="Key generators">
+      {#each generators as g (g.name)}
+        <li class="flex items-center gap-2 px-3 py-2">
+          <span class="min-w-0 flex-1 truncate text-sm">{g.name}</span>
+          <span class="shrink-0 text-xs text-faint">
+            {hostsUsing($hosts, g.name).length} host{hostsUsing($hosts, g.name).length === 1 ? '' : 's'}
+          </span>
+          <button type="button" class={iconBtn} title="Edit {g.name}" aria-label="Edit {g.name}" onclick={() => (dialog = { kind: 'edit', generator: g })}>
+            <Icon name="edit" size={14} />
+          </button>
+          <button type="button" class={iconBtn} title="Delete {g.name}" aria-label="Delete {g.name}" onclick={() => (dialog = { kind: 'delete', name: g.name })}>
+            <Icon name="trash" size={14} />
+          </button>
+        </li>
+      {:else}
+        <li class="px-3 py-2 text-xs text-muted">No key generators yet.</li>
+      {/each}
+    </ul>
+    <Button variant="ghost" onclick={() => (dialog = { kind: 'add' })}>
+      <Icon name="plus" size={14} /> Add key generator
+    </Button>
+  {/if}
+</div>
+
+{#if dialog?.kind === 'add'}
+  <KeyGeneratorEditor
+    mode="add"
+    initial={emptyGeneratorFields()}
+    taken={generators.map((g) => g.name)}
+    onSubmit={(g) => save(null, g)}
+    onCancel={() => (dialog = null)}
+  />
+{:else if dialog?.kind === 'edit'}
+  {@const from = dialog.generator}
+  <KeyGeneratorEditor
+    mode="edit"
+    initial={{ name: from.name, code: from.code, credential: from.credential }}
+    taken={generators.filter((g) => g.name !== from.name).map((g) => g.name)}
+    onSubmit={(g) => save(from.name, g)}
+    onCancel={() => (dialog = null)}
+  />
+{:else if dialog?.kind === 'delete'}
+  {@const name = dialog.name}
+  {@const using = hostsUsing($hosts, name)}
+  <Modal label="Delete key generator" onClose={() => (dialog = null)}>
+    <div class="space-y-3 px-5 py-4">
+      <h2 class="text-sm font-semibold">Delete key generator</h2>
+      <p class="text-sm text-muted">Delete “{name}”?</p>
+      {#if using.length > 0}
+        <p class="text-sm text-muted">
+          {using.join(', ')}
+          {using.length === 1 ? 'uses' : 'use'} it, and will log in without it from now on.
+        </p>
+      {/if}
+      <div class="flex justify-end gap-2 pt-1">
+        <Button variant="ghost" onclick={() => (dialog = null)}>Cancel</Button>
+        <Button variant="primary" onclick={() => remove(name)}>Delete</Button>
+      </div>
+    </div>
+  </Modal>
+{/if}
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/Settings.svelte b/crates/omnyssh-gui/ui/src/lib/screens/Settings.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/Settings.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/Settings.svelte
@@ -6,6 +6,7 @@
   import ShortcutSettings from './ShortcutSettings.svelte';
   import SchemePicker from './SchemePicker.svelte';
   import GlobalHotkeySetting from './GlobalHotkeySetting.svelte';
+  import KeyGeneratorSettings from './KeyGeneratorSettings.svelte';
   import { theme } from '$lib/stores/theme';
   import { streamerMode } from '$lib/stores/streamer';
   import {
@@ -194,6 +195,12 @@
       </div>
     </Surface>
 
+    <!-- Key generators -->
+    <Surface class="p-5">
+      <h2 class="mb-3 text-sm font-semibold">Key generators</h2>
+      <KeyGeneratorSettings />
+    </Surface>
+
     <!-- Terminal colors -->
     <Surface class="p-5">
       <h2 class="mb-3 text-sm font-semibold">Terminal colors</h2>
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.ts b/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/keyGenForm.ts
@@ -0,0 +1,46 @@
+// The key generator editor's checks and the test panel's verdict, kept out of the
+// component so they are unit-testable.
+import type { HostDto, KeyGeneratorDto } from '$lib/bindings';
+
+export interface GeneratorFields {
+  name: string;
+  code: string;
+  credential: string;
+}
+
+export function emptyGeneratorFields(): GeneratorFields {
+  return {
+    name: '',
+    code: 'async function generateKey(deviceInfo, address) {\n  // deviceInfo: { credential, mac, revision? }\n  return { privateKey: \'\' };\n}\n',
+    credential: ''
+  };
+}
+
+export type GeneratorResult = { ok: true; generator: KeyGeneratorDto } | { ok: false; error: string };
+
+/** Name present and free (`taken`: the other generators' names), credential JSON or blank. */
+export function generatorFromFields(f: GeneratorFields, taken: readonly string[]): GeneratorResult {
+  const name = f.name.trim();
+  if (!name) return { ok: false, error: 'Name is required' };
+  if (taken.includes(name)) return { ok: false, error: `A key generator named "${name}" already exists` };
+  if (f.credential.trim() !== '') {
+    try {
+      JSON.parse(f.credential);
+    } catch {
+      return { ok: false, error: 'The credential is not valid JSON' };
+    }
+  }
+  return { ok: true, generator: { name, code: f.code, credential: f.credential.trim() } };
+}
+
+/** Whether the code's own fingerprint agrees with the key's, `SHA256:` or not. */
+export function fingerprintsAgree(fromKey: string, fromCode: string | undefined): boolean {
+  if (fromCode === undefined) return true;
+  const bare = (f: string) => f.trim().replace(/^SHA256:/, '').replace(/=+$/, '');
+  return bare(fromKey) === bare(fromCode);
+}
+
+/** The hosts that name generator `name`. */
+export function hostsUsing(hosts: readonly HostDto[], name: string): string[] {
+  return hosts.filter((h) => h.keyGenerator === name).map((h) => h.name);
+}
diff --git a/crates/omnyssh-gui/ui/src/lib/stores/keyGenerators.ts b/crates/omnyssh-gui/ui/src/lib/stores/keyGenerators.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/stores/keyGenerators.ts
@@ -0,0 +1,17 @@
+import { writable } from 'svelte/store';
+import type { KeyGeneratorsDto } from '$lib/bindings';
+import { listKeyGenerators } from '$lib/ipc/commands';
+import { lastError } from './notifications';
+
+/** The key generators' switch and the generators (`key_generators.toml`); null until
+ *  first loaded. Read by the settings page and the host form. */
+export const keyGenerators = writable<KeyGeneratorsDto | null>(null);
+
+/** Reads them from the backend; a failure is reported and leaves the last copy. */
+export async function loadKeyGenerators(): Promise<void> {
+  try {
+    keyGenerators.set(await listKeyGenerators());
+  } catch (e) {
+    lastError.set(`Key generators: ${e instanceof Error ? e.message : String(e)}`);
+  }
+}
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npm run check 2>&1 | tail -1; npx vitest run src/lib/screens/keyGenForm.test.ts 2>&1 | grep "Tests "; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: svelte-check 无错误；`5 passed`；e2e `4 passed`。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
git commit -m "feat(ui): key generators in Settings"
```

---

### Task 7: 前端：主机表单和卡片标签

**Files:** Modify `crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts`、`hostForm.test.ts`、`HostEditor.svelte`、`Dashboard.svelte`、`e2e/key-generators.spec.ts`

`formToInput(fields, withKeyGen)`：`withKeyGen` 为假（总开关关）时不发 `keyGen`。Dashboard 打开时加载生成器，把名字列表（关时为 `null`）传给三个主机编辑框。新增 2 个 e2e 用例：关时表单和卡片都没有相关内容、保存不带 `keyGen`；开时卡片显示 `gen`，给主机选生成器（缺 MAC 时报错）并保存，复制主机沿用 MAC 和 revision。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r7-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r7-test.patch && git apply /tmp/r7-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
--- a/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
+++ b/crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
@@ -247,3 +247,50 @@
   expect(await calls(page, 'delete_key_generator')).toEqual([{ name: 'vendor-b' }]);
   await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('No key generators yet.');
 });
+
+const SET_UP = { ...CAM, keyGenerator: 'vendor-a', deviceMac: '30:ae:7b:eb:17:82', deviceRevision: 2 };
+
+test('while generators are off the host form and the cards show nothing of them', async ({ page }) => {
+  await boot(page, { ...ON, enabled: false }, [SET_UP, WEB]);
+  await expect(page.getByText('gen', { exact: true })).toHaveCount(0);
+  await page.getByRole('button', { name: 'Edit cam' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit host' });
+  await expect(editor.getByLabel('Key generator')).toHaveCount(0);
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect.poll(() => calls(page, 'save_host')).toHaveLength(1);
+  // Left out, so the backend keeps the host's generator.
+  expect('keyGen' in ((await calls(page, 'save_host'))[0].input as object)).toBe(false);
+});
+
+test('a host picks a generator with its MAC and revision; a duplicate keeps them', async ({ page }) => {
+  await boot(page, ON, [SET_UP, WEB]);
+  await expect(page.getByText('gen', { exact: true })).toHaveCount(1);
+  await expect(page.getByText('gen', { exact: true })).toHaveAttribute('title', /vendor-a/);
+
+  await page.getByRole('button', { name: 'Edit web' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit host' });
+  await expect(editor.getByLabel('MAC')).toHaveCount(0);
+  await editor.getByLabel('Key generator').selectOption('vendor-a');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect(editor.getByText('MAC is required for a key generator')).toBeVisible();
+  await editor.getByLabel('MAC').fill('30:ae:7b:eb:17:99');
+  await editor.getByLabel('Revision').fill('3');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect.poll(() => calls(page, 'save_host')).toHaveLength(1);
+  expect(((await calls(page, 'save_host'))[0].input as { keyGen: unknown }).keyGen).toEqual({
+    generator: 'vendor-a',
+    mac: '30:ae:7b:eb:17:99',
+    revision: 3
+  });
+
+  await page.getByRole('button', { name: 'Duplicate cam' }).click();
+  const copy = page.getByRole('dialog', { name: 'Add host' });
+  await expect(copy.getByLabel('MAC')).toHaveValue('30:ae:7b:eb:17:82');
+  await copy.getByRole('button', { name: 'Add host' }).click();
+  await expect.poll(() => calls(page, 'duplicate_host')).toHaveLength(1);
+  expect(((await calls(page, 'duplicate_host'))[0].input as { keyGen: unknown }).keyGen).toEqual({
+    generator: 'vendor-a',
+    mac: '30:ae:7b:eb:17:82',
+    revision: 2
+  });
+});
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
@@ -358,3 +358,43 @@
     expect(out.ok && [out.input.uploadProtocol, out.input.uploadDir]).toEqual(['zmodem', undefined]);
   });
 });
+
+describe('formToInput — key generator', () => {
+  const cam = (over: Partial<HostFormFields> = {}) =>
+    fields({ name: 'cam', hostname: '192.168.11.1', keyGenerator: 'vendor-a', deviceMac: ' 30:ae:7b:eb:17:82 ', ...over });
+
+  it('leaves keyGen out while generators are off, so the backend keeps what is set', () => {
+    const out = formToInput(cam());
+    expect(out.ok && 'keyGen' in out.input).toBe(false);
+  });
+
+  it('sends the generator, the MAC and the revision while they are on', () => {
+    const out = formToInput(cam({ deviceRevision: ' 2 ' }), true);
+    expect(out.ok && out.input.keyGen).toEqual({ generator: 'vendor-a', mac: '30:ae:7b:eb:17:82', revision: 2 });
+    const blank = formToInput(cam(), true);
+    expect(blank.ok && blank.input.keyGen).toEqual({ generator: 'vendor-a', mac: '30:ae:7b:eb:17:82', revision: null });
+  });
+
+  it('sends no generator as all three cleared', () => {
+    const out = formToInput(cam({ keyGenerator: '' }), true);
+    expect(out.ok && out.input.keyGen).toEqual({ generator: null, mac: null, revision: null });
+  });
+
+  it('wants a MAC and a whole revision', () => {
+    expect(formToInput(cam({ deviceMac: ' ' }), true)).toEqual({ ok: false, error: 'MAC is required for a key generator' });
+    expect(formToInput(cam({ deviceRevision: '1.5' }), true)).toEqual({
+      ok: false,
+      error: "Revision must be a whole number, got '1.5'"
+    });
+    expect(formToInput(cam({ deviceRevision: '4294967296' }), true).ok).toBe(false);
+  });
+
+  it('seeds the form from the host and keeps it on a duplicate', () => {
+    const h = host({ keyGenerator: 'vendor-a', deviceMac: '30:ae:7b:eb:17:82', deviceRevision: 3 });
+    const f = formFromHost(h);
+    expect([f.keyGenerator, f.deviceMac, f.deviceRevision]).toEqual(['vendor-a', '30:ae:7b:eb:17:82', '3']);
+    const dup = formForDuplicate(h, []);
+    expect([dup.keyGenerator, dup.deviceMac, dup.deviceRevision]).toEqual(['vendor-a', '30:ae:7b:eb:17:82', '3']);
+    expect([emptyForm().keyGenerator, formFromHost(host({})).deviceRevision]).toEqual(['', '']);
+  });
+});
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/screens/hostForm.test.ts 2>&1 | grep -E "Tests |×" | head -6; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `formToInput — key generator` 里 4 个失败（`4 failed | 43 passed`）；e2e 1 个失败（`a host picks a generator…`，`toHaveCount` 不符），5 个通过——"关时什么都没有"那个在实现前就成立。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/r7-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/r7-impl.patch && git apply /tmp/r7-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte b/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
@@ -14,6 +14,7 @@
   import { spawnSession } from '$lib/stores/navigation';
   import { streamerMode, displayHostname } from '$lib/stores/streamer';
   import { hosts } from '$lib/stores/hosts';
+  import { keyGenerators, loadKeyGenerators } from '$lib/stores/keyGenerators';
   import { lastError } from '$lib/stores/notifications';
   import { isWindows } from '$lib/platform';
   import { serialDevices, reloadSerialDevices } from '$lib/stores/serialDevices';
@@ -153,7 +154,12 @@
   // Saved serial devices (Windows only, like the Serial entry): cards after the hosts.
   onMount(() => {
     if (isWindows) void reloadSerialDevices();
+    void loadKeyGenerators();
   });
+  // The host form's key generator fields: shown only while generators are on.
+  const generatorNames = $derived(
+    $keyGenerators?.enabled ? $keyGenerators.generators.map((g) => g.name) : null
+  );
   const devices = $derived(isWindows ? $serialDevices : []);
   const visibleDevices = $derived(filterSerialDevices(devices, query));
   const deviceNames = $derived(devices.map((d) => d.name));
@@ -297,6 +303,14 @@
                       title="Imported from ~/.ssh/config — editing saves your own copy, which takes priority"
                     >
                       ssh config
+                    </span>
+                  {/if}
+                  {#if $keyGenerators?.enabled && card.host.keyGenerator}
+                    <span
+                      class="shrink-0 rounded-full border border-default px-1.5 py-0.5 text-[10px] text-faint"
+                      title="Its key comes from key generator {card.host.keyGenerator} first"
+                    >
+                      gen
                     </span>
                   {/if}
                   <!-- Auth-state reflection (tech-gui.md §4.2): key-only once password
@@ -535,7 +549,13 @@
 </section>
 
 {#if dialog?.kind === 'add'}
-  <HostEditor mode="add" initial={emptyForm()} onSubmit={submit} onCancel={() => (dialog = null)} />
+  <HostEditor
+    mode="add"
+    initial={emptyForm()}
+    keyGenerators={generatorNames}
+    onSubmit={submit}
+    onCancel={() => (dialog = null)}
+  />
 {:else if dialog?.kind === 'edit'}
   {@const host = dialog.host}
   <HostEditor
@@ -543,6 +563,7 @@
     initial={formFromHost(host)}
     previousName={host.name}
     imported={host.source === 'sshConfig'}
+    keyGenerators={generatorNames}
     onSubmit={submit}
     onCancel={() => (dialog = null)}
   />
@@ -552,6 +573,7 @@
     mode="add"
     initial={formForDuplicate(host, $hosts.map((h) => h.name))}
     copiedFrom={host.name}
+    keyGenerators={generatorNames}
     onSubmit={(input) => submitDuplicate(host.name, input)}
     onCancel={() => (dialog = null)}
   />
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte b/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
@@ -17,6 +17,7 @@
     previousName,
     imported = false,
     copiedFrom,
+    keyGenerators = null,
     onSubmit,
     onCancel
   }: {
@@ -27,6 +28,9 @@
     imported?: boolean;
     /** Duplicating this host: the blank login fields keep its values — say so. */
     copiedFrom?: string;
+    /** The key generators' names while they are switched on; null hides the fields,
+     *  and the save then leaves the host's settings as they are. */
+    keyGenerators?: readonly string[] | null;
     onSubmit: (input: HostInputDto, previousName: string | undefined) => Promise<void>;
     onCancel: () => void;
   } = $props();
@@ -46,7 +50,7 @@
   onMount(() => (mode === 'add' ? nameEl : hostnameEl)?.focus());
 
   async function save(): Promise<void> {
-    const result = formToInput(fields);
+    const result = formToInput(fields, keyGenerators !== null);
     if (!result.ok) {
       error = result.error;
       return;
@@ -65,6 +69,13 @@
   // On edit the DTO omits identity/password (§3.4), so the fields start blank and mean
   // "keep the stored value"; on add they mean "none".
   const secretHint = $derived(mode === 'edit' ? 'Leave blank to keep the current value' : undefined);
+
+  // One a host names but the list no longer has stays pickable rather than vanishing.
+  const generatorChoices = $derived(
+    keyGenerators === null
+      ? []
+      : [...new Set([...keyGenerators, ...(fields.keyGenerator ? [fields.keyGenerator] : [])])]
+  );
 
   const label = 'block space-y-1 text-xs font-medium text-muted';
   const forwardGrid = 'grid grid-cols-[8.5rem,1fr,4.5rem,1.75rem] items-center gap-2';
@@ -207,6 +218,33 @@
           Files dropped on a terminal are sent to <span class="font-mono">rz</span> in its shell — the host needs lrzsz.
         {/if}
       </p>
+
+      {#if keyGenerators !== null}
+        <div class="grid grid-cols-[1fr,1fr,6rem] gap-3">
+          <label class={label}>
+            <span>Key generator</span>
+            <Select bind:value={fields.keyGenerator} class={field}>
+              <option value="">None</option>
+              {#each generatorChoices as name (name)}<option value={name}>{name}</option>{/each}
+            </Select>
+          </label>
+          {#if fields.keyGenerator}
+            <label class={label}>
+              <span>MAC</span>
+              <input bind:value={fields.deviceMac} class={field} placeholder="30:ae:7b:eb:17:82" autocomplete="off" />
+            </label>
+            <label class={label}>
+              <span>Revision</span>
+              <input bind:value={fields.deviceRevision} class={field} placeholder="Default" inputmode="numeric" />
+            </label>
+          {/if}
+        </div>
+        {#if fields.keyGenerator}
+          <p class="text-xs text-faint">
+            The key it makes for this device is tried first; the login goes on as usual if it does not get in.
+          </p>
+        {/if}
+      {/if}
 
       <div class="flex items-center justify-between gap-4 border-t border-default pt-3.5">
         <div class="min-w-0">
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
@@ -6,6 +6,7 @@
 import type {
   HostDto,
   HostInputDto,
+  HostKeyGenDto,
   HostUploadProtocolDto,
   LocalForwardDto,
   MonitorModeDto
@@ -39,6 +40,11 @@
   uploadProtocol: HostUploadProtocolDto;
   /** Where SFTP uploads go: blank means the home folder. Only read for `sftp`. */
   uploadDir: string;
+  /** The key generator's name; blank for none. */
+  keyGenerator: string;
+  deviceMac: string;
+  /** Blank leaves it to the generator's code. */
+  deviceRevision: string;
 }
 
 export function emptyForm(): HostFormFields {
@@ -59,7 +65,10 @@
     tunnelAutostart: false,
     forwardAgent: false,
     uploadProtocol: 'zmodem',
-    uploadDir: ''
+    uploadDir: '',
+    keyGenerator: '',
+    deviceMac: '',
+    deviceRevision: ''
   };
 }
 
@@ -97,7 +106,10 @@
     tunnelAutostart: h.tunnelAutostart,
     forwardAgent: h.forwardAgent,
     uploadProtocol: h.uploadProtocol ?? 'zmodem',
-    uploadDir: h.uploadDir ?? ''
+    uploadDir: h.uploadDir ?? '',
+    keyGenerator: h.keyGenerator ?? '',
+    deviceMac: h.deviceMac ?? '',
+    deviceRevision: h.deviceRevision == null ? '' : String(h.deviceRevision)
   };
 }
 
@@ -181,7 +193,7 @@
  *  dropped to `undefined` when empty (so the wire form stays sparse, §4.1). `proxyJump`
  *  is not surfaced by the form (parity with the TUI, which sets it `None`) — `save_host`
  *  preserves any existing value across an edit. */
-export function formToInput(f: HostFormFields): HostFormResult {
+export function formToInput(f: HostFormFields, withKeyGen = false): HostFormResult {
   const name = f.name.trim();
   if (!name) return { ok: false, error: 'Name cannot be empty' };
   const hostname = f.hostname.trim();
@@ -214,6 +226,9 @@
 
   const localForwards = parseForwards(f.forwards);
   if (typeof localForwards === 'string') return { ok: false, error: localForwards };
+
+  const keyGen = withKeyGen ? parseKeyGen(f) : undefined;
+  if (typeof keyGen === 'string') return { ok: false, error: keyGen };
 
   const identityFile = f.identityFile.trim();
   const password = f.password.trim();
@@ -239,7 +254,22 @@
       // Always sent: an omitted protocol means "keep" to the backend, and takes the
       // folder with it. A folder nobody can see (ZMODEM) is cleared.
       uploadProtocol: f.uploadProtocol,
-      uploadDir: f.uploadProtocol === 'sftp' ? f.uploadDir.trim() || undefined : undefined
-    }
-  };
-}
+      uploadDir: f.uploadProtocol === 'sftp' ? f.uploadDir.trim() || undefined : undefined,
+      // Only while key generators are on: left out, the backend keeps what is set.
+      ...(keyGen ? { keyGen } : {})
+    }
+  };
+}
+
+/** The key generator fields, or an error. No generator sends none of them. */
+function parseKeyGen(f: HostFormFields): HostKeyGenDto | string {
+  const generator = f.keyGenerator.trim();
+  if (!generator) return { generator: null, mac: null, revision: null };
+  const mac = f.deviceMac.trim();
+  if (!mac) return 'MAC is required for a key generator';
+  const revision = f.deviceRevision.trim();
+  if (revision !== '' && (!/^\d+$/.test(revision) || Number(revision) > 0xffffffff)) {
+    return `Revision must be a whole number, got '${revision}'`;
+  }
+  return { generator, mac, revision: revision === '' ? null : Number(revision) };
+}
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npm run check 2>&1 | tail -1; npx vitest run 2>&1 | grep "Tests "; npx playwright test e2e/key-generators.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) |Error:"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: svelte-check 无错误；vitest `509 passed`；e2e `6 passed`。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/screens crates/omnyssh-gui/ui/e2e/key-generators.spec.ts
git commit -m "feat(ui): key generator fields in the host form, and a gen tag on the card"
```

---

### Task 8: 全量检查和手动验证

- [ ] **Step 1: Rust 全量检查**

```bash
nohup sh -c 'cargo fmt --all -- --check && echo FMT_OK; cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep -E "^(error|warning)" | head; cargo test --workspace 2>&1 | grep -E "test result|FAILED|panicked"; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`。

Expected: `FMT_OK`；clippy 没有输出；所有 `test result: ok`，没有 `FAILED`。`crates/omnyssh-core/tests/tunnel.rs` 在基线上就偶发失败（与本计划无关），如果只有它失败，单独重跑一次确认，并在报告里写明。`upload_pty` 的测试需要 lrzsz：没装时它们会因为找不到 `rz` 失败，属于环境问题，在报告里写明即可。

- [ ] **Step 2: 前端全量检查**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npm run check 2>&1 | tail -1; npx vitest run 2>&1 | grep "Tests "; for i in 1 2; do npx playwright test --reporter=line 2>&1 | grep -E "passed|failed|flaky"; done; echo EXIT=$?' > /tmp/r.log 2>&1 &
```
然后反复 `tail -20 /tmp/r.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: svelte-check `0 ERRORS 0 WARNINGS`；vitest 509 个通过；两遍 Playwright 都是 118 个通过。

- [ ] **Step 3: 手动验证清单（需要人在真实应用里做，执行者只需在报告里列出来）**

在仓库根目录 `cargo tauri dev` 启动应用：
1. 设置 › Key generators：开关默认关闭；仪表盘卡片和主机表单里都没有生成器相关内容。
2. 打开开关，新增生成器：粘贴你们的 `generateKey` 代码和凭证 JSON；在测试区填一台真实设备的 MAC（和 revision）点 Run，显示的指纹和设备上 `~/.ssh/authorized_keys` 里那把公钥的指纹（`ssh-keygen -lf`）一致，没有"不一致"提示。
3. 给这台设备的主机选生成器、填 MAC，主机表单里不填密码，打开终端：直接登录成功；卡片上有 `gen` 标签。
4. 把 MAC 改错一位再连：生成的密钥被拒，如果主机有密码就用密码登录，没有就提示输入密码。
5. 关闭开关后再连同一台设备：不再用生成的密钥（和功能上线前一样）；打开开关后恢复。
6. 生成器改名：主机表单里显示新名字；删除生成器：确认框列出在用的主机，删除后这些主机的生成器字段清空。
7. 在 TUI（`cargo run -p omnyssh`）里编辑这台主机的其它字段并保存，再回到桌面版：生成器、MAC、revision 都还在。
8. `ls -l ~/.config/omnyssh/key_generators.toml`（macOS 在 `~/Library/Application Support/omnyssh/`）：权限是 `-rw-------`。

- [ ] **Step 4: 如果 Step 1、2 有失败**

按失败信息修，修完重跑，再单独提交（`fix: …`）。没有失败就跳过。
