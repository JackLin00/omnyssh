# P. 拖放上传：后端接入 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把计划 O 的上传引擎接进串口线程和 SSH pump，加上主机/串口设备的上传协议配置、SFTP 上传，以及 GUI 的三个命令（`terminal_upload`、`serial_upload`、`upload_cancel`）和两个事件（`upload-progress`、`upload-finished`）。做完后后端完整可用，只差前端的拖放和界面（下一份计划 **Q. 拖放上传：前端**）。

**Architecture:**
- core 新增 `transfer::ActiveUpload`：串口和 SSH 共用的"一次上传"驱动。它启动协议、把进度节流到 100 ms、决定何时退出传输模式（发完，或结束后最多再等 2 s），并在结束时生成一行结果，作为终端输出显示（也因此进入会话日志）。
- `serial.rs` 和 `ssh/pty.rs` 的控制消息新增 `Upload { preamble, upload }` 和 `CancelUpload`。收到 `Upload` 时在同一步里发出 `preamble`（ZMODEM 是 `rz -b\r`）并进入传输模式：之后收到的字节都交给引擎，不显示、不进日志。
- GUI 新增 `src/upload.rs`（预检查、SFTP 目标路径、SFTP 逐个文件推进、"谁在上传"的登记）和 `src/commands/upload.rs`（三个命令）；`state.rs`、`bridge.rs` 只加接线。SFTP 上传走每个终端单独的一条 SFTP 连接，第一次 SFTP 拖放时打开，终端关闭时关掉。

**Tech Stack:** Rust（tokio、russh 0.46、Tauri 2 + tauri-specta）、计划 O 的 `omnyssh_core::transfer`。不新增依赖。

**Spec:** `docs/superpowers/specs/2026-11-07-drag-drop-upload-design.md`（含"接入时的注意事项"一节）

**命令约定（沙箱限制，务必遵守）：**
- 前台命令如果 60–100 秒没有输出会被沙箱杀掉。所有 cargo 命令都用 `nohup sh -c '…; echo EXIT=$?' > /tmp/p.log 2>&1 &` 放到后台，再反复 `tail -20 /tmp/p.log` 直到出现 `EXIT=`。
- cargo 命令在仓库根目录执行，前端命令在 `crates/omnyssh-gui/ui` 下执行。磁盘很紧张：**不要**另设 `CARGO_TARGET_DIR`，也不要新建 worktree 去编译。
- **补丁的用法：** 每个代码步骤给出一个补丁。用 Write 工具把它原样存到指定的 `/tmp/…` 文件（不要重排、不要去掉行尾空格），然后在仓库根目录 `git apply --check <文件> && git apply <文件>`。`--check` 失败就停下来报告，不要手工"修"补丁。
- **已知的不稳定测试：** `crates/omnyssh-core/tests/tunnel.rs` 在这台机器上本来就偶发失败（未改动的基线上 10 遍失败 3 遍，每次是不同的用例，有时是 30 s 等待超时），和这份计划无关。全量测试里只有它失败时，单独重跑 `cargo test -p omnyssh-core --test tunnel`，通过即可；其他测试失败一律当真。
- `crates/omnyssh-gui/ui/src/lib/bindings.ts` 是生成文件，补丁里已经包含它的改动（有两行带行尾空格，`git apply` 会提示 whitespace，属于正常）。如果它对不上（`committed_bindings_are_in_sync` 失败），重新生成：在仓库根目录后台运行 `nohup sh -c 'xvfb-run -a timeout 180 cargo run -p omnyssh-gui; true' > /tmp/p-run.log 2>&1 &`，轮询到 bindings 里出现补丁加入的名字（Task 1 是 `SerialUploadProtocolDto`，Task 6 是 `uploadFinished`），再 `pkill -x OmnySSH || true`。

---

## 实现中确定的设计决定

> **执行记录**：执行中代码审查又发现并修复了若干问题（ZMODEM 取消会丢弃排队的文件数据；关闭会话时取消上传；登录中被拒的上传在各种结局下都报结束；失效的会话拒绝上传；SFTP 的特殊文件、断线重连、竞态、节流、主机快照、连接期间取消），上面的决定已按最终行为更新。各任务里的补丁是最初版本，最终代码以仓库为准。

> 写这份计划时，整套改动已经在仓库的临时副本里完整实现并验证过：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check`、`npm run check`、`npx vitest run`（449 个）和全部 97 个 Playwright 用例都通过；对 lrzsz 0.12.21 的互通测试（计划 O 的 3 个，加上这里新增的经 SSH 的 1 个）也都通过。之后又把改动按下面 6 个任务在另一个副本里逐个重放、逐个验证，下面的补丁就是从这串提交生成的，并在第三个副本上按顺序应用验证过：每个测试补丁之后编译失败（失败信息写在各任务里），每个实现补丁之后测试全部通过。

1. **拆成两份计划**：P 只做后端（core 接入 + GUI 命令、事件、配置、SFTP），全部能用单元测试和集成测试覆盖；拖放交互、遮罩、进度条、表单字段放在计划 Q。P 做完后前端代码不需要任何改动就能继续编译和通过测试。
2. **结果行（取代 spec 里两种文案）**：结束时终端里显示一行英文，前面先换行：
   - `[upload] uploaded foo.bin (1.2 MB)`、`[upload] uploaded 3 files (4.5 MB)`、`[upload] uploaded 2 of 3 files, 1 skipped by the receiver`、`[upload] foo.bin was skipped by the receiver`
   - `[upload] failed: <原因>`、`[upload] cancelled`
   - 这一行作为普通终端输出走现有的输出路径，所以**自动进入会话日志**；协议字节从不作为输出，所以不会进日志。spec 里的中文终端提示和日志里的 `[upload] … ok|failed` 合并成了这一行（界面文字都是英文）。
   - `upload-finished` 的 `message` 就是这行 `[upload] ` 后面的内容（失败时不带 `failed: `）。
   - SFTP 失败的 `message` 是 `<文件名>: <错误>`，SFTP 连接断开是 `the SFTP connection was lost`。登录中被拒的上传（决定 6）只发 `upload-finished`，终端里没有结果行。
3. **`ActiveUpload` 放在 core**（`transfer/active.rs`），串口和 SSH 共用，进度节流（100 ms）也在这里做，而不是 spec 说的在 GUI 里按会话节流。
4. **结束后的排空**：任何结束（完成、失败、取消）之后，会话继续发送引擎还没交出的字节（取消序列、ZMODEM 的 `OO`），发完就退出传输模式；最多等 2 s，线路卡住时丢掉剩下的。spec 只对取消规定了 2 s，这里对所有结束都适用，否则失败后卡住的线路会让会话永远停在传输模式。
5. **连接断开与关闭**（执行中修订）：
   - 串口读写出错、SSH 通道 EOF/关闭时，如果有 ZMODEM/YMODEM/XMODEM 上传在进行，先报 `the connection was lost`（结果行照常显示），再报会话结束。
   - SFTP 上传时终端远端退出：SFTP 连接随终端关掉，不发 `upload-finished`，只有 `terminal-exited`。
   - 用户关闭标签：串口和 SSH 都先向设备发取消序列（SSH 最多等 500 ms）。之后串口线程不报结束；SSH pump 报 `cancelled`，但 GUI 已注销该会话，事件被丢弃。两者对前端都表现为"什么都没有"。
   - 在已经结束的会话上发起上传，命令直接返回错误（`the port is closed` / `the terminal is closed`）。
6. **登录中的终端**：SSH 还在密码提示阶段时收到的 `Upload` 不执行，在提示阶段结束时报失败 `the terminal was still logging in`（不论登录成功、失败还是标签被关），所以也有 `upload-finished`（不打印结果行）。没有密码提示时，连接期间收到的 `Upload` 会排队到 shell 打开后执行；如果连接失败，它不会有 `upload-finished`，只有 `terminal-exited`。
7. **传输期间的键盘输入**：按 spec 照常转发（串口进 `pending`，SSH 发给通道）；由前端在传输期间禁用输入。
8. **配置字段**（core）：`Host.upload_protocol: HostUploadProtocol { Zmodem, Sftp }`、`Host.upload_dir: Option<String>`、`SerialDevice.upload_protocol: transfer::Protocol`；默认值（ZMODEM、未设置）都不写进配置文件。TUI 没有这些表单项，编辑主机时原样保留。
9. **配置字段**（GUI DTO）：TS 里全部是可选字段，现有前端代码和测试不受影响。
   - `HostDto.uploadProtocol?`（总是发送）、`HostDto.uploadDir?`。
   - `HostInputDto.uploadProtocol?`、`uploadDir?`：**省略协议表示保持原值**（和 `monitoring` 一样），协议和目录一起保留；空白目录表示 home。
   - 复制主机时，省略协议则沿用源主机的协议和目录。
   - `SerialDeviceDto.uploadProtocol?`：省略表示 ZMODEM。现在的前端表单还不带这个字段，所以在计划 Q 之前用界面保存设备会把它重置为 ZMODEM；只有手改过 `serial.toml` 的设备会受影响。
10. **预检查在 GUI 命令里**（发送任何东西之前）：空列表、文件夹、XMODEM 多文件、ZMODEM 大于 4 GiB、ZMODEM/YMODEM 文件名超过 256 字节、这个会话已经有上传在进行。对应英文消息见 `src/upload.rs`。XMODEM 不发文件名，所以不检查名字长度。
11. **SFTP**：
    - 每个终端第一次 SFTP 拖放时打开一条 SFTP 连接（和 SFTP 标签页一样会弹密码、主机密钥确认），之后复用；终端关闭或远端退出时关掉。
    - 一次传一个文件，上一个完成才发下一个，所以失败或取消会停掉后面的文件；**取消在当前文件传完后生效**（core 的 SFTP 上传不能中途停）。
    - 目标路径：`upload_dir` 未设置或 `~` 时为文件名本身（SFTP 的相对路径以 home 为基准），`~/x` 变成 `x/文件名`，其他按原样拼接。
    - 结果行由 GUI 打进终端（xterm 和会话日志），不经过远端 shell。
    - （执行中补充）连接用的是终端打开时的主机快照，之后改名或编辑主机不影响。连接建立期间取消，`terminal_upload` 返回 `Err("cancelled")`。某个文件失败后丢弃这条连接，下次拖放重新连（可能再弹一次密码）。每条连接带代号，旧连接的事件不会影响新连接。进度按文件节流到 100 ms（每个文件的第一个进度总会发出）。
    - （执行中补充）只接受普通文件：FIFO、设备文件等直接拒绝（`<名字> is not a regular file`），否则打开时会一直阻塞，或像 `/dev/zero` 那样永远读不完。
12. **事件 `upload-progress`** 比 spec 多一个 `waiting` 字段：接收端还没开始时为 `true`（其他字段为空），前端据此显示"等待接收端"。`fileIndex` 从 0 开始。只有 ZMODEM/YMODEM/XMODEM 会报 `waiting`。
13. **TUI** 不发起上传，只需要在 `CoreEvent` 的 match 里忽略 `Upload`。

---

## 给计划 Q 的接口

- 命令（`commands` 里）：
  - `terminalUpload(sessionId, protocol: "zmodem" | "sftp", paths: string[])`
  - `serialUpload(sessionId, protocol: "zmodem" | "ymodem" | "xmodem", paths: string[])`
  - `uploadCancel(sessionId)`
  - 返回 `error` 时没有向终端或设备发送任何东西（预检查失败、已有上传、会话已关闭）；SFTP 例外：连接可能已经打开并保留。
  - `terminalUpload` 返回 `error` 且 `message === "cancelled"`：SFTP 连接建立期间用户取消了，按取消处理，不是失败（此时没有 `upload-finished`，终端里也没有结果行）。SFTP 首次上传要先建立连接（可能弹密码、主机密钥确认），命令可能挂起很久，前端这期间显示"连接中"。
  - 返回 `ok` 后，上传以下面任一为结束：一个 `upload-finished`；该终端的 `terminal-exited`（远端退出时，SFTP 上传和极少数竞态不会再有 `upload-finished`）；串口标签的 `onExit`；标签被用户关闭（关闭后不会再有任何上传事件）。前端要把这几种都当作结束，并忽略未知会话的 `upload-finished`。
- 事件：
  - `uploadProgress`：`{ sessionId, waiting, fileIndex, fileCount, fileName, sent, total }`，每个会话最多 100 ms 一次。
  - `uploadFinished`：`{ sessionId, ok, message }`。`message` 原样显示，不要解析。
  - SFTP 上传没有 `waiting: true` 的进度；第一个进度事件在第一个文件开始传数据时才到。取消 SFTP 时，如果当前已是最后一个文件，结果是 `ok: true`。
- 进度注意：ZMODEM 的 `sent` 可能重复或倒退；跳过或空文件之后，进度会停在上一个文件的 100%，直到下一个文件开始。
- 配置：`HostDto.uploadProtocol` / `uploadDir`、`HostInputDto.uploadProtocol` / `uploadDir`、`SerialDeviceDto.uploadProtocol`。
- 终端里的结果行由后端输出，前端不需要自己打印。
- 键盘输入只在 ZMODEM/YMODEM/XMODEM 上传期间禁用（从调用命令到结束），SFTP 不禁用。
- `HostInputDto` 省略 `uploadProtocol` 时协议和目录一起保留，所以表单发 `uploadDir` 时必须同时发协议。串口设备表单要带上 `uploadProtocol`，否则保存会重置为 ZMODEM。

---

## 文件结构

| 文件 | 任务 | 职责 |
|---|---|---|
| `crates/omnyssh-core/src/ssh/client.rs` | 1 | `HostUploadProtocol`，`Host.upload_protocol` / `upload_dir` |
| `crates/omnyssh-core/src/config/serial_devices.rs` | 1 | `SerialDevice.upload_protocol` |
| `crates/omnyssh/src/app/host.rs` | 1 | TUI 编辑主机时保留上传设置 |
| `crates/omnyssh-gui/src/dto.rs`、`src/commands/hosts.rs` | 1 | 配置 DTO；省略协议时保留原值 |
| `crates/omnyssh-core/src/transfer/active.rs`（新建） | 2 | `ActiveUpload`、`UploadEvent`、`UploadResult`、`Finished`、`human_size` |
| `crates/omnyssh-core/src/transfer/mod.rs` | 2 | 声明并导出 `active` |
| `crates/omnyssh-core/src/serial.rs` | 3 | 串口线程的传输模式 |
| `crates/omnyssh-gui/src/commands/serial.rs` | 1、3、6 | 测试夹具；`SerialOutput::Upload` 的路由；接到事件 |
| `crates/omnyssh-core/src/ssh/pty.rs`、`src/event.rs` | 4 | SSH pump 的传输模式，`CoreEvent::Upload` |
| `crates/omnyssh/src/app/mod.rs` | 4 | TUI 忽略 `CoreEvent::Upload` |
| `crates/omnyssh-core/tests/upload_pty.rs`（新建） | 4 | 进程内 SSH 服务端上的端到端测试（含一个需要 lrzsz 的 `#[ignore]` 用例） |
| `crates/omnyssh-gui/src/upload.rs`（新建） | 5 | 预检查、SFTP 目标路径、SFTP 逐个推进、`Uploads` 登记 |
| `crates/omnyssh-gui/src/events.rs` | 5 | `UploadProgress`、`UploadFinished` |
| `crates/omnyssh-gui/src/commands/upload.rs`（新建） | 6 | 三个命令 |
| `crates/omnyssh-gui/src/state.rs`、`src/bridge.rs`、`src/main.rs`、`src/commands/mod.rs` | 6 | 接线、注册 |
| `crates/omnyssh-gui/ui/src/lib/bindings.ts` | 1、6 | 重新生成 |

lrzsz：互通测试需要 `rz`。本机没有时不需要 root，可以解包：`mkdir -p /tmp/lrzsz && cd /tmp/lrzsz && apt-get download lrzsz && dpkg-deb -x lrzsz_*.deb . && ls usr/bin`，之后用 `LRZSZ_DIR=/tmp/lrzsz/usr/bin`。

---

### Task 1: 上传协议配置

**Files:** core `ssh/client.rs`、`config/serial_devices.rs`；TUI `app/host.rs`；GUI `dto.rs`、`commands/hosts.rs`、`commands/serial.rs`（测试夹具）、`ui/src/lib/bindings.ts`

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p1-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p1-test.patch && git apply /tmp/p1-test.patch`。

```diff
diff --git a/crates/omnyssh-core/src/config/serial_devices.rs b/crates/omnyssh-core/src/config/serial_devices.rs
--- a/crates/omnyssh-core/src/config/serial_devices.rs
+++ b/crates/omnyssh-core/src/config/serial_devices.rs
@@ -141,6 +141,7 @@
             flow_control: FlowControl::Hardware,
             enter: LineEnding::Crlf,
             notes: Some("bench".to_string()),
+            upload_protocol: Protocol::Ymodem,
         }
     }
 
@@ -183,8 +184,26 @@
                 flow_control: FlowControl::None,
                 enter: LineEnding::Cr,
                 notes: None,
+                upload_protocol: Protocol::Zmodem,
             }]
         );
+    }
+
+    #[test]
+    fn the_default_upload_protocol_is_not_written() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("serial.toml");
+        let mut board = device("board");
+        board.upload_protocol = Protocol::Zmodem;
+        save_to(&path, &[board]).unwrap();
+        let text = std::fs::read_to_string(&path).unwrap();
+        assert!(!text.contains("upload_protocol"), "{text}");
+
+        let mut board = device("board");
+        board.upload_protocol = Protocol::Xmodem;
+        save_to(&path, &[board]).unwrap();
+        let text = std::fs::read_to_string(&path).unwrap();
+        assert!(text.contains("upload_protocol = \"xmodem\""), "{text}");
     }
 
     #[test]
diff --git a/crates/omnyssh-gui/src/commands/hosts.rs b/crates/omnyssh-gui/src/commands/hosts.rs
--- a/crates/omnyssh-gui/src/commands/hosts.rs
+++ b/crates/omnyssh-gui/src/commands/hosts.rs
@@ -237,7 +237,8 @@
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::dto::MonitorModeDto;
+    use crate::dto::{HostUploadProtocolDto, MonitorModeDto};
+    use omnyssh_core::ssh::client::HostUploadProtocol;
     use omnyssh_core::ssh::client::{HostSource, MonitorMode};
 
     fn input(name: &str) -> HostInputDto {
@@ -256,6 +257,8 @@
             local_forwards: vec![],
             tunnel_autostart: false,
             forward_agent: false,
+            upload_protocol: None,
+            upload_dir: None,
         }
     }
 
@@ -390,6 +393,26 @@
     }
 
     #[test]
+    fn upsert_keeps_upload_settings_the_payload_left_out() {
+        let mut hosts = vec![Host {
+            name: "nas".to_string(),
+            upload_protocol: HostUploadProtocol::Sftp,
+            upload_dir: Some("/srv/drop".to_string()),
+            source: HostSource::Manual,
+            ..Host::default()
+        }];
+        upsert(&mut hosts, input("nas"), None);
+        assert_eq!(hosts[0].upload_protocol, HostUploadProtocol::Sftp);
+        assert_eq!(hosts[0].upload_dir.as_deref(), Some("/srv/drop"));
+
+        let mut back = input("nas");
+        back.upload_protocol = Some(HostUploadProtocolDto::Zmodem);
+        upsert(&mut hosts, back, None);
+        assert_eq!(hosts[0].upload_protocol, HostUploadProtocol::Zmodem);
+        assert_eq!(hosts[0].upload_dir, None);
+    }
+
+    #[test]
     fn upsert_applies_a_monitoring_mode_the_payload_carries() {
         let mut hosts = vec![Host {
             name: "fw".to_string(),
diff --git a/crates/omnyssh-gui/src/commands/serial.rs b/crates/omnyssh-gui/src/commands/serial.rs
--- a/crates/omnyssh-gui/src/commands/serial.rs
+++ b/crates/omnyssh-gui/src/commands/serial.rs
@@ -218,6 +218,7 @@
             flow_control: FlowControl::None,
             enter: LineEnding::Cr,
             notes: None,
+            upload_protocol: Default::default(),
         }
     }
 
diff --git a/crates/omnyssh-gui/src/dto.rs b/crates/omnyssh-gui/src/dto.rs
--- a/crates/omnyssh-gui/src/dto.rs
+++ b/crates/omnyssh-gui/src/dto.rs
@@ -1004,6 +1004,8 @@
             local_forwards: vec![],
             tunnel_autostart: false,
             forward_agent: false,
+            upload_protocol: None,
+            upload_dir: None,
         }
     }
 
@@ -1031,6 +1033,55 @@
     }
 
     #[test]
+    fn upload_settings_cross_both_ways() {
+        let mut input = full_input();
+        input.upload_protocol = Some(HostUploadProtocolDto::Sftp);
+        input.upload_dir = Some(" ~/drop ".to_string());
+        let host = Host::from(input);
+        assert_eq!(host.upload_protocol, HostUploadProtocol::Sftp);
+        assert_eq!(host.upload_dir.as_deref(), Some("~/drop"));
+        let json = serde_json::to_value(HostDto::from(&host)).unwrap();
+        assert_eq!(json["uploadProtocol"], "sftp");
+        assert_eq!(json["uploadDir"], "~/drop");
+
+        // A payload from a form that predates upload settings still reads.
+        let old: HostInputDto = serde_json::from_value(serde_json::json!({
+            "name": "a", "hostname": "h", "user": "u", "port": 22, "tags": [],
+            "localForwards": [], "tunnelAutostart": false, "forwardAgent": false
+        }))
+        .unwrap();
+        assert_eq!(old.upload_protocol, None);
+    }
+
+    #[test]
+    fn a_serial_device_keeps_its_upload_protocol() {
+        let device = SerialDevice {
+            name: "board".to_string(),
+            port: "COM3".to_string(),
+            baud_rate: 115_200,
+            data_bits: 8,
+            parity: Parity::None,
+            stop_bits: StopBits::One,
+            flow_control: FlowControl::None,
+            enter: LineEnding::Cr,
+            notes: None,
+            upload_protocol: Protocol::Ymodem,
+        };
+        let dto = SerialDeviceDto::from(&device);
+        assert_eq!(dto.upload_protocol, SerialUploadProtocolDto::Ymodem);
+        assert_eq!(SerialDevice::from(dto), device);
+        // Left out means ZMODEM.
+        let old: SerialDeviceDto = serde_json::from_value(serde_json::json!({
+            "name": "b",
+            "config": {"port": "COM1", "baudRate": 9600, "dataBits": 8, "parity": "none",
+                       "stopBits": "one", "flowControl": "none"},
+            "enter": "cr", "notes": null
+        }))
+        .unwrap();
+        assert_eq!(old.upload_protocol, SerialUploadProtocolDto::Zmodem);
+    }
+
+    #[test]
     fn host_input_keeps_the_password_backend_side() {
         // The password rides inbound into the backend `Host`, then the outbound
         // `HostDto` must drop it: it never reaches the webview (§3.4).
@@ -1060,8 +1111,11 @@
             local_forwards: vec![],
             tunnel_autostart: false,
             forward_agent: false,
+            upload_protocol: None,
+            upload_dir: Some("  ".to_string()),
         });
         assert!(host.identity_file.is_none());
+        assert!(host.upload_dir.is_none(), "a blank upload dir means home");
         assert!(host.password.is_none());
         assert!(host.proxy_jump.is_none());
         assert!(host.notes.is_none());
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test --workspace --no-run 2>&1 | grep -E "^error" | sort | uniq -c; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 编译失败：`SerialDevice` 没有 `upload_protocol` 字段、找不到 `Protocol`（core 的测试），以及 GUI 测试里 `HostInputDto` 缺字段、找不到 `HostUploadProtocolDto` / `SerialUploadProtocolDto` / `HostUploadProtocol`。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p1-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p1-impl.patch && git apply /tmp/p1-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/config/serial_devices.rs b/crates/omnyssh-core/src/config/serial_devices.rs
--- a/crates/omnyssh-core/src/config/serial_devices.rs
+++ b/crates/omnyssh-core/src/config/serial_devices.rs
@@ -8,6 +8,7 @@
 use serde::{Deserialize, Serialize};
 
 use crate::serial::{FlowControl, Parity, SerialConfig, StopBits};
+use crate::transfer::Protocol;
 use crate::utils::platform;
 
 /// What the Enter key sends in a serial terminal.
@@ -42,6 +43,14 @@
     pub enter: LineEnding,
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub notes: Option<String>,
+    /// How a file dropped on this device's tab is sent.
+    #[serde(default, skip_serializing_if = "is_zmodem")]
+    pub upload_protocol: Protocol,
+}
+
+/// Lets the default protocol stay out of `serial.toml`.
+fn is_zmodem(protocol: &Protocol) -> bool {
+    *protocol == Protocol::Zmodem
 }
 
 impl SerialDevice {
diff --git a/crates/omnyssh-core/src/ssh/client.rs b/crates/omnyssh-core/src/ssh/client.rs
--- a/crates/omnyssh-core/src/ssh/client.rs
+++ b/crates/omnyssh-core/src/ssh/client.rs
@@ -37,6 +37,24 @@
     /// Lets the default stay out of `hosts.toml` entirely.
     fn is_ssh(&self) -> bool {
         matches!(self, Self::Ssh)
+    }
+}
+
+/// How a file dropped on a host's terminal is sent.
+#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
+#[serde(rename_all = "lowercase")]
+pub enum HostUploadProtocol {
+    /// To `rz` on the host, through the terminal itself.
+    #[default]
+    Zmodem,
+    /// Over a separate SFTP connection, into `upload_dir`.
+    Sftp,
+}
+
+impl HostUploadProtocol {
+    /// Lets the default stay out of `hosts.toml` entirely.
+    fn is_zmodem(&self) -> bool {
+        matches!(self, Self::Zmodem)
     }
 }
 
@@ -103,6 +121,12 @@
     /// root on the host can use it while a terminal is open, so it is opt-in.
     #[serde(default, skip_serializing_if = "std::ops::Not::not")]
     pub forward_agent: bool,
+    /// How a file dropped on this host's terminal is sent.
+    #[serde(default, skip_serializing_if = "HostUploadProtocol::is_zmodem")]
+    pub upload_protocol: HostUploadProtocol,
+    /// Where SFTP uploads go: `~`, `~/dir` or an absolute path. Unset means `~`.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub upload_dir: Option<String>,
 
     // -----------------------------------------------------------------------
     // Auto SSH Key Setup metadata
@@ -158,6 +182,8 @@
             local_forwards: Vec::new(),
             tunnel_autostart: false,
             forward_agent: false,
+            upload_protocol: HostUploadProtocol::default(),
+            upload_dir: None,
             key_setup_date: None,
             password_auth_disabled: None,
         }
diff --git a/crates/omnyssh-gui/src/commands/hosts.rs b/crates/omnyssh-gui/src/commands/hosts.rs
--- a/crates/omnyssh-gui/src/commands/hosts.rs
+++ b/crates/omnyssh-gui/src/commands/hosts.rs
@@ -129,6 +129,9 @@
     // An omitted monitoring mode means "unchanged", not "back to SSH" — losing it
     // would silently start logging in to a device chosen for reachability only.
     let monitoring_given = input.monitoring.is_some();
+    // Likewise an omitted upload protocol: a form that predates it must not reset
+    // an SFTP host to ZMODEM.
+    let upload_given = input.upload_protocol.is_some();
     let mut host = Host::from(input);
     match hosts.iter().position(|h| h.name == host.name) {
         Some(i) => {
@@ -136,6 +139,10 @@
             if !monitoring_given {
                 host.monitoring = existing.monitoring;
                 host.monitor_port = existing.monitor_port;
+            }
+            if !upload_given {
+                host.upload_protocol = existing.upload_protocol;
+                host.upload_dir = existing.upload_dir.clone();
             }
             host.password = host.password.or_else(|| existing.password.clone());
             host.identity_file = host
@@ -176,7 +183,12 @@
 /// monitoring mode means "unchanged", like `upsert` — not silently back to SSH.
 fn duplicate(source: &Host, input: HostInputDto) -> Host {
     let monitoring_given = input.monitoring.is_some();
+    let upload_given = input.upload_protocol.is_some();
     let mut host = Host::from(input);
+    if !upload_given {
+        host.upload_protocol = source.upload_protocol;
+        host.upload_dir = source.upload_dir.clone();
+    }
     host.password = host.password.or_else(|| source.password.clone());
     host.identity_file = host.identity_file.or_else(|| source.identity_file.clone());
     host.proxy_jump = host.proxy_jump.or_else(|| source.proxy_jump.clone());
diff --git a/crates/omnyssh-gui/src/dto.rs b/crates/omnyssh-gui/src/dto.rs
--- a/crates/omnyssh-gui/src/dto.rs
+++ b/crates/omnyssh-gui/src/dto.rs
@@ -12,11 +12,14 @@
     DetectedService, KeyFingerprint, MetricValue, Metrics, ProcessInfo, ServiceKind, ServiceMetric,
 };
 use omnyssh_core::serial::{FlowControl, Parity, PortInfo, SerialConfig, StopBits};
-use omnyssh_core::ssh::client::{ConnectionStatus, Host, HostSource, MonitorMode};
+use omnyssh_core::ssh::client::{
+    ConnectionStatus, Host, HostSource, HostUploadProtocol, MonitorMode,
+};
 use omnyssh_core::ssh::host_key::Decision;
 use omnyssh_core::ssh::key_setup::KeySetupStep;
 use omnyssh_core::ssh::sftp::FileEntry;
 use omnyssh_core::ssh::tunnel::{LocalForward, TunnelStatus};
+use omnyssh_core::transfer::Protocol;
 use omnyssh_core::update::UpdateInfo;
 
 /// Host origin, mirrors `omnyssh_core::ssh::client::HostSource`.
@@ -50,6 +53,63 @@
         match mode {
             MonitorModeDto::Ssh => Self::Ssh,
             MonitorModeDto::TcpPort => Self::TcpPort,
+        }
+    }
+}
+
+/// How a file dropped on a host's terminal is sent, mirrors `HostUploadProtocol`.
+#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
+#[serde(rename_all = "camelCase")]
+pub enum HostUploadProtocolDto {
+    #[default]
+    Zmodem,
+    Sftp,
+}
+
+impl From<HostUploadProtocol> for HostUploadProtocolDto {
+    fn from(protocol: HostUploadProtocol) -> Self {
+        match protocol {
+            HostUploadProtocol::Zmodem => Self::Zmodem,
+            HostUploadProtocol::Sftp => Self::Sftp,
+        }
+    }
+}
+
+impl From<HostUploadProtocolDto> for HostUploadProtocol {
+    fn from(protocol: HostUploadProtocolDto) -> Self {
+        match protocol {
+            HostUploadProtocolDto::Zmodem => Self::Zmodem,
+            HostUploadProtocolDto::Sftp => Self::Sftp,
+        }
+    }
+}
+
+/// How a file dropped on a serial tab is sent, mirrors `transfer::Protocol`.
+#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
+#[serde(rename_all = "camelCase")]
+pub enum SerialUploadProtocolDto {
+    #[default]
+    Zmodem,
+    Ymodem,
+    Xmodem,
+}
+
+impl From<Protocol> for SerialUploadProtocolDto {
+    fn from(protocol: Protocol) -> Self {
+        match protocol {
+            Protocol::Zmodem => Self::Zmodem,
+            Protocol::Ymodem => Self::Ymodem,
+            Protocol::Xmodem => Self::Xmodem,
+        }
+    }
+}
+
+impl From<SerialUploadProtocolDto> for Protocol {
+    fn from(protocol: SerialUploadProtocolDto) -> Self {
+        match protocol {
+            SerialUploadProtocolDto::Zmodem => Self::Zmodem,
+            SerialUploadProtocolDto::Ymodem => Self::Ymodem,
+            SerialUploadProtocolDto::Xmodem => Self::Xmodem,
         }
     }
 }
@@ -77,6 +137,12 @@
     pub local_forwards: Vec<LocalForwardDto>,
     pub tunnel_autostart: bool,
     pub forward_agent: bool,
+    // Optional on the wire (`serde(default)`) so frontends that predate drag-and-drop
+    // upload still type-check; always sent.
+    #[serde(default)]
+    pub upload_protocol: HostUploadProtocolDto,
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub upload_dir: Option<String>,
 }
 
 /// One `ssh -L` rule (tech-gui.md §4.1): listen on `bindAddress:bindPort` here and
@@ -135,6 +201,12 @@
     pub local_forwards: Vec<LocalForwardDto>,
     pub tunnel_autostart: bool,
     pub forward_agent: bool,
+    /// Left out means unchanged, like `monitoring`; `uploadDir` goes with it.
+    #[serde(default)]
+    pub upload_protocol: Option<HostUploadProtocolDto>,
+    /// Blank means the home directory.
+    #[serde(default)]
+    pub upload_dir: Option<String>,
 }
 
 /// What this desktop allows the tray (tech-gui.md §4.2 `set_tray_behavior`): an icon
@@ -421,6 +493,9 @@
     pub config: SerialConfigDto,
     pub enter: LineEndingDto,
     pub notes: Option<String>,
+    // Optional on the wire, like `HostDto::upload_protocol`; left out means ZMODEM.
+    #[serde(default)]
+    pub upload_protocol: SerialUploadProtocolDto,
 }
 
 impl From<&SerialDevice> for SerialDeviceDto {
@@ -452,6 +527,7 @@
                 LineEnding::Crlf => LineEndingDto::Crlf,
             },
             notes: d.notes.clone(),
+            upload_protocol: d.upload_protocol.into(),
         }
     }
 }
@@ -477,6 +553,7 @@
                 .notes
                 .map(|n| n.trim().to_string())
                 .filter(|n| !n.is_empty()),
+            upload_protocol: dto.upload_protocol.into(),
         }
     }
 }
@@ -507,6 +584,8 @@
             local_forwards: host.local_forwards.iter().map(Into::into).collect(),
             tunnel_autostart: host.tunnel_autostart,
             forward_agent: host.forward_agent,
+            upload_protocol: host.upload_protocol.into(),
+            upload_dir: host.upload_dir.clone(),
         }
     }
 }
@@ -597,6 +676,8 @@
             local_forwards: dto.local_forwards.into_iter().map(Into::into).collect(),
             tunnel_autostart: dto.tunnel_autostart,
             forward_agent: dto.forward_agent,
+            upload_protocol: dto.upload_protocol.map(Into::into).unwrap_or_default(),
+            upload_dir: non_empty(dto.upload_dir.map(|d| d.trim().to_string())),
             key_setup_date: None,
             password_auth_disabled: None,
         }
diff --git a/crates/omnyssh-gui/ui/src/lib/bindings.ts b/crates/omnyssh-gui/ui/src/lib/bindings.ts
--- a/crates/omnyssh-gui/ui/src/lib/bindings.ts
+++ b/crates/omnyssh-gui/ui/src/lib/bindings.ts
@@ -757,7 +757,7 @@
  * (tech-gui.md §3.4). `hasKey` reports whether an identity file is configured;
  * the key path itself never crosses the boundary.
  */
-export type HostDto = { name: string; hostname: string; user: string; port: number; tags: string[]; notes?: string | null; source: HostSourceDto; hasKey: boolean; passwordAuthDisabled?: boolean | null; monitoring: MonitorModeDto; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean }
+export type HostDto = { name: string; hostname: string; user: string; port: number; tags: string[]; notes?: string | null; source: HostSourceDto; hasKey: boolean; passwordAuthDisabled?: boolean | null; monitoring: MonitorModeDto; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean; uploadProtocol?: HostUploadProtocolDto; uploadDir?: string | null }
 /**
  * Inbound host form payload for `save_host` (tech-gui.md §4.1, Stage 4.1). Always
  * builds a **manual** `Host`: editing an SSH-config import saves a copy that shadows
@@ -766,7 +766,15 @@
  * travel back out: the outbound `HostDto` omits both (§3.4). Inbound only, so it
  * derives `Deserialize` (not `Serialize`).
  */
-export type HostInputDto = { name: string; hostname: string; user: string; port: number; identityFile?: string | null; password?: string | null; proxyJump?: string | null; tags: string[]; notes?: string | null; monitoring?: MonitorModeDto | null; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean }
+export type HostInputDto = { name: string; hostname: string; user: string; port: number; identityFile?: string | null; password?: string | null; proxyJump?: string | null; tags: string[]; notes?: string | null; monitoring?: MonitorModeDto | null; monitorPort?: number | null; localForwards: LocalForwardDto[]; tunnelAutostart: boolean; forwardAgent: boolean; 
+/**
+ * Left out means unchanged, like `monitoring`; `uploadDir` goes with it.
+ */
+uploadProtocol?: HostUploadProtocolDto | null; 
+/**
+ * Blank means the home directory.
+ */
+uploadDir?: string | null }
 /**
  * A host key no longer matches `known_hosts` on a connection the user started
  * (a terminal or files); `jumpFor` names the host a bastion was on the way to.
@@ -786,6 +794,10 @@
  * A host's connection status changed (tech-gui.md §4.3).
  */
 export type HostStatusChanged = { hostName: string; status: ConnectionStatusDto }
+/**
+ * How a file dropped on a host's terminal is sent, mirrors `HostUploadProtocol`.
+ */
+export type HostUploadProtocolDto = "zmodem" | "sftp"
 /**
  * Full host list broadcast. Emitted by `reload_hosts` after refreshing the
  * cache; the bridge does not map `HostsLoaded` (tech-gui.md §3.4).
@@ -897,7 +909,7 @@
 /**
  * A saved serial device, shown as a dashboard card.
  */
-export type SerialDeviceDto = { name: string; config: SerialConfigDto; enter: LineEndingDto; notes: string | null }
+export type SerialDeviceDto = { name: string; config: SerialConfigDto; enter: LineEndingDto; notes: string | null; uploadProtocol?: SerialUploadProtocolDto }
 /**
  * Why a serial tab's port closed on its own (adapter unplugged, driver error).
  * Sent once on the tab's exit channel; a user-initiated close sends nothing.
@@ -907,6 +919,10 @@
  * A serial port present on this machine.
  */
 export type SerialPortDto = { name: string; description: string | null }
+/**
+ * How a file dropped on a serial tab is sent, mirrors `transfer::Protocol`.
+ */
+export type SerialUploadProtocolDto = "zmodem" | "ymodem" | "xmodem"
 /**
  * A service detected on a host with its quick-scan metrics (tech-gui.md §4.1).
  */
diff --git a/crates/omnyssh/src/app/host.rs b/crates/omnyssh/src/app/host.rs
--- a/crates/omnyssh/src/app/host.rs
+++ b/crates/omnyssh/src/app/host.rs
@@ -293,6 +293,8 @@
             local_forwards,
             tunnel_autostart,
             forward_agent,
+            upload_protocol: Default::default(),
+            upload_dir: None,
             key_setup_date: None,
             password_auth_disabled: None,
         })
@@ -577,6 +579,11 @@
                         // Nor has IdentitiesOnly, and losing it brings back a login
                         // that offers every agent key.
                         host.identities_only = old_host.is_some_and(|h| h.identities_only);
+                        // Nor have the desktop's upload settings.
+                        if let Some(old) = old_host {
+                            host.upload_protocol = old.upload_protocol;
+                            host.upload_dir = old.upload_dir.clone();
+                        }
 
                         // An import is adopted under the name it was imported by; a copy
                         // already adopted keeps the one it carries. Dropping it brings the
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test --workspace 2>&1 | grep -E "test result|FAILED|panicked|^error"; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 所有 `test result` 都是 `ok`，包括新增的 `the_default_upload_protocol_is_not_written`（core）、`upload_settings_cross_both_ways`、`a_serial_device_keeps_its_upload_protocol`、`upsert_keeps_upload_settings_the_payload_left_out`（GUI）和 `committed_bindings_are_in_sync`。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core/src crates/omnyssh/src crates/omnyssh-gui/src crates/omnyssh-gui/ui/src/lib/bindings.ts
git commit -m "feat: upload protocol settings for hosts and serial devices"
```

---

### Task 2: `ActiveUpload`：会话里的一次上传

**Files:** Create `crates/omnyssh-core/src/transfer/active.rs`；Modify `crates/omnyssh-core/src/transfer/mod.rs`

测试里的 `Fake` 是一个由测试控制状态和输出的 `Upload`，用来单独测驱动逻辑；最后一个测试用真实的 XMODEM 走一遍。

- [ ] **Step 1: 先加测试（新文件只有测试模块，`mod.rs` 先声明并导出）**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p2-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p2-test.patch && git apply /tmp/p2-test.patch`。

```diff
diff --git a/crates/omnyssh-core/src/transfer/active.rs b/crates/omnyssh-core/src/transfer/active.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-core/src/transfer/active.rs
@@ -0,0 +1,221 @@
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use std::sync::{Arc, Mutex};
+
+    /// An upload whose state and output the test sets.
+    #[derive(Default)]
+    struct Script {
+        state: Option<UploadState>,
+        to_send: Vec<Vec<u8>>,
+        leftover: Vec<u8>,
+        received: Vec<u8>,
+        ticks: usize,
+        cancelled: bool,
+    }
+
+    struct Fake(Arc<Mutex<Script>>);
+
+    impl Upload for Fake {
+        fn receive(&mut self, data: &[u8]) {
+            self.0.lock().unwrap().received.extend_from_slice(data);
+        }
+        fn transmit(&mut self, out: &mut Vec<u8>) {
+            let mut s = self.0.lock().unwrap();
+            if !s.to_send.is_empty() {
+                let chunk = s.to_send.remove(0);
+                out.extend_from_slice(&chunk);
+            }
+        }
+        fn tick(&mut self, _now: Instant) {
+            self.0.lock().unwrap().ticks += 1;
+        }
+        fn cancel(&mut self) {
+            let mut s = self.0.lock().unwrap();
+            s.cancelled = true;
+            s.state = Some(UploadState::Cancelled);
+            s.to_send = vec![b"CANCEL".to_vec()];
+        }
+        fn state(&self) -> UploadState {
+            self.0
+                .lock()
+                .unwrap()
+                .state
+                .clone()
+                .unwrap_or(UploadState::Waiting)
+        }
+        fn leftover(&mut self) -> Vec<u8> {
+            std::mem::take(&mut self.0.lock().unwrap().leftover)
+        }
+    }
+
+    fn fake(files: &[(&str, u64)]) -> (ActiveUpload, Arc<Mutex<Script>>) {
+        let script = Arc::new(Mutex::new(Script::default()));
+        let files = files.iter().map(|(n, s)| (n.to_string(), *s)).collect();
+        (
+            ActiveUpload::new(Box::new(Fake(Arc::clone(&script))), files),
+            script,
+        )
+    }
+
+    fn sending(sent: u64) -> UploadState {
+        UploadState::Sending {
+            file_index: 0,
+            file_count: 1,
+            file_name: "a.bin".to_string(),
+            sent,
+            total: 100,
+        }
+    }
+
+    #[test]
+    fn receive_ticks_before_handing_the_bytes_over() {
+        let (mut up, script) = fake(&[("a.bin", 100)]);
+        up.receive(b"C", Instant::now());
+        let s = script.lock().unwrap();
+        assert_eq!(s.ticks, 1);
+        assert_eq!(s.received, b"C");
+    }
+
+    #[test]
+    fn pump_fills_only_an_empty_buffer() {
+        let (mut up, script) = fake(&[("a.bin", 100)]);
+        script.lock().unwrap().to_send = vec![b"one".to_vec(), b"two".to_vec()];
+        let now = Instant::now();
+        let mut pending = b"unsent".to_vec();
+        assert_eq!(up.pump(now, &mut pending), None);
+        assert_eq!(pending, b"unsent", "nothing added while bytes wait");
+        pending.clear();
+        assert_eq!(up.pump(now, &mut pending), None);
+        assert_eq!(pending, b"one");
+    }
+
+    #[test]
+    fn a_finished_upload_ends_once_everything_is_sent() {
+        let (mut up, script) = fake(&[("a.bin", 2048)]);
+        {
+            let mut s = script.lock().unwrap();
+            s.state = Some(UploadState::Done {
+                sent_files: 1,
+                skipped: 0,
+            });
+            s.to_send = vec![b"OO".to_vec()];
+            s.leftover = b"$ ".to_vec();
+        }
+        let now = Instant::now();
+        let mut pending = Vec::new();
+        assert_eq!(up.pump(now, &mut pending), None, "OO goes out first");
+        assert_eq!(pending, b"OO");
+        pending.clear();
+        let end = up.pump(now, &mut pending).expect("ended");
+        assert_eq!(
+            end.result,
+            UploadResult {
+                ok: true,
+                message: "uploaded a.bin (2.0 KB)".to_string()
+            }
+        );
+        assert_eq!(end.output, b"$ \r\n[upload] uploaded a.bin (2.0 KB)\r\n");
+    }
+
+    #[test]
+    fn a_stalled_line_is_given_up_on_after_the_drain_limit() {
+        let (mut up, script) = fake(&[("a.bin", 100)]);
+        up.cancel();
+        assert!(script.lock().unwrap().cancelled);
+        let t = Instant::now();
+        let mut pending = Vec::new();
+        assert_eq!(up.pump(t, &mut pending), None);
+        assert_eq!(pending, b"CANCEL");
+        // The line takes nothing.
+        assert_eq!(up.pump(t + Duration::from_secs(1), &mut pending), None);
+        let end = up.pump(t + DRAIN_LIMIT, &mut pending).expect("given up");
+        assert!(pending.is_empty(), "the unsent rest is dropped");
+        assert_eq!(end.result.message, "cancelled");
+        assert_eq!(end.output, b"\r\n[upload] cancelled\r\n");
+    }
+
+    #[test]
+    fn a_failure_reads_as_failed_with_its_reason() {
+        let (mut up, script) = fake(&[("a.bin", 100)]);
+        script.lock().unwrap().state = Some(UploadState::Failed("too many retries".to_string()));
+        let end = up.pump(Instant::now(), &mut Vec::new()).unwrap();
+        assert!(!end.result.ok);
+        assert_eq!(end.output, b"\r\n[upload] failed: too many retries\r\n");
+    }
+
+    #[test]
+    fn results_name_the_files() {
+        let (up, _) = fake(&[("a.bin", 100)]);
+        assert_eq!(up.done_message(0, 1), "a.bin was skipped by the receiver");
+        let (up, _) = fake(&[("a", 1024), ("b", 1024), ("c", 1024)]);
+        assert_eq!(up.done_message(3, 0), "uploaded 3 files (3.0 KB)");
+        assert_eq!(
+            up.done_message(2, 1),
+            "uploaded 2 of 3 files, 1 skipped by the receiver"
+        );
+    }
+
+    #[test]
+    fn progress_is_reported_when_it_changes_at_most_every_100_ms() {
+        let (mut up, script) = fake(&[("a.bin", 100)]);
+        let t = Instant::now();
+        assert_eq!(up.progress(t), Some(UploadState::Waiting));
+        assert_eq!(up.progress(t + PROGRESS_INTERVAL), None, "unchanged");
+        script.lock().unwrap().state = Some(sending(10));
+        assert_eq!(up.progress(t + Duration::from_millis(50)), None, "too soon");
+        assert_eq!(up.progress(t + PROGRESS_INTERVAL), Some(sending(10)));
+        script.lock().unwrap().state = Some(UploadState::Cancelled);
+        assert_eq!(
+            up.progress(t + 10 * PROGRESS_INTERVAL),
+            None,
+            "final states end through pump"
+        );
+    }
+
+    #[test]
+    fn a_lost_connection_fails_the_upload() {
+        let (up, script) = fake(&[("a.bin", 100)]);
+        script.lock().unwrap().leftover = b"partial".to_vec();
+        let end = up.lost();
+        assert!(!end.result.ok);
+        assert_eq!(
+            end.output,
+            b"partial\r\n[upload] failed: the connection was lost\r\n"
+        );
+    }
+
+    #[test]
+    fn sizes_read_in_binary_units() {
+        assert_eq!(human_size(0), "0 B");
+        assert_eq!(human_size(1023), "1023 B");
+        assert_eq!(human_size(1024), "1.0 KB");
+        assert_eq!(human_size(1_258_291), "1.2 MB");
+        assert_eq!(human_size(5 << 30), "5.0 GB");
+    }
+
+    #[test]
+    fn a_real_protocol_runs_through_the_driver() {
+        // XMODEM to a receiver that only ever says C then ACKs.
+        let file = UploadFile {
+            name: "a.bin".to_string(),
+            size: 128,
+            modified: None,
+            reader: Box::new(std::io::Cursor::new(vec![7u8; 128])),
+        };
+        let t = Instant::now();
+        let mut up = ActiveUpload::start(Protocol::Xmodem, vec![file], t);
+        let mut pending = Vec::new();
+        up.receive(b"C", t);
+        assert_eq!(up.pump(t, &mut pending), None);
+        assert_eq!(pending[0], 0x01, "a 128-byte block (SOH)");
+        pending.clear();
+        up.receive(&[0x06], t); // ACK the block
+        assert_eq!(up.pump(t, &mut pending), None);
+        assert_eq!(pending, [0x04], "EOT");
+        pending.clear();
+        up.receive(&[0x06], t);
+        let end = up.pump(t, &mut pending).expect("done");
+        assert!(end.result.ok, "{:?}", end.result);
+    }
+}
diff --git a/crates/omnyssh-core/src/transfer/mod.rs b/crates/omnyssh-core/src/transfer/mod.rs
--- a/crates/omnyssh-core/src/transfer/mod.rs
+++ b/crates/omnyssh-core/src/transfer/mod.rs
@@ -14,8 +14,11 @@
 
 use serde::{Deserialize, Serialize};
 
+mod active;
 mod xmodem;
 mod zmodem;
+
+pub use active::{human_size, ActiveUpload, Finished, UploadEvent, UploadResult};
 
 /// How much one [`Upload::transmit`] call adds at most. The caller only asks
 /// again once it has sent it all, so this bounds what sits in its buffer.
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer::active 2>&1 | grep -E "^error" | sort | uniq -c; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 编译失败：`pub use active::{…}` 指向的 `active` 模块里还只有测试，报 `cannot find … ActiveUpload`、`UploadResult`、`human_size` 等。

- [ ] **Step 3: 实现（补在测试模块前面）**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p2-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p2-impl.patch && git apply /tmp/p2-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/transfer/active.rs b/crates/omnyssh-core/src/transfer/active.rs
--- a/crates/omnyssh-core/src/transfer/active.rs
+++ b/crates/omnyssh-core/src/transfer/active.rs
@@ -1,3 +1,205 @@
+//! One upload inside a session loop: the part the serial thread and the SSH
+//! pump share. It starts the protocol, throttles progress reports, decides
+//! when the session may leave upload mode, and writes the result line the
+//! terminal shows (and the session log records) at the end.
+
+use std::time::{Duration, Instant};
+
+use super::{start, Protocol, Upload, UploadFile, UploadState};
+
+/// Progress goes out at most this often.
+const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
+
+/// How long an upload that has ended may keep sending what it still has
+/// queued (a cancel sequence, ZMODEM's `OO`) before the session drops it. A
+/// stalled line would otherwise keep the session in upload mode for good.
+const DRAIN_LIMIT: Duration = Duration::from_secs(2);
+
+/// How an upload ended.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct UploadResult {
+    pub ok: bool,
+    /// `uploaded foo.bin (1.2 MB)`, `cancelled`, or why it failed.
+    pub message: String,
+}
+
+impl UploadResult {
+    /// The line the terminal shows at the end, starting on a fresh line: what
+    /// the device sent last may have left the cursor anywhere.
+    pub fn line(&self) -> String {
+        if self.ok || self.message == CANCELLED {
+            format!("\r\n[upload] {}\r\n", self.message)
+        } else {
+            format!("\r\n[upload] failed: {}\r\n", self.message)
+        }
+    }
+}
+
+const CANCELLED: &str = "cancelled";
+
+/// What a session reports about its upload.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum UploadEvent {
+    /// Waiting for the receiver, or sending; never a final state.
+    Progress(UploadState),
+    Finished(UploadResult),
+}
+
+/// The end of an upload: what to show in the terminal (the device's last
+/// output, then the result line), and the result.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct Finished {
+    pub output: Vec<u8>,
+    pub result: UploadResult,
+}
+
+pub struct ActiveUpload {
+    upload: Box<dyn Upload>,
+    /// Name and size of each file, for the result line.
+    files: Vec<(String, u64)>,
+    reported: Option<UploadState>,
+    reported_at: Option<Instant>,
+    /// Set when the upload is first seen ended.
+    drain_until: Option<Instant>,
+}
+
+impl ActiveUpload {
+    /// Starts sending `files`. The caller has checked them first: not empty,
+    /// one file for XMODEM (see [`start`]).
+    pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Self {
+        let names = files.iter().map(|f| (f.name.clone(), f.size)).collect();
+        Self::new(start(protocol, files, now), names)
+    }
+
+    fn new(upload: Box<dyn Upload>, files: Vec<(String, u64)>) -> Self {
+        Self {
+            upload,
+            files,
+            reported: None,
+            reported_at: None,
+            drain_until: None,
+        }
+    }
+
+    /// Bytes the device sent.
+    pub fn receive(&mut self, data: &[u8], now: Instant) {
+        self.upload.tick(now);
+        self.upload.receive(data);
+    }
+
+    /// Call regularly (every 100 ms or so) while uploading.
+    pub fn tick(&mut self, now: Instant) {
+        self.upload.tick(now);
+    }
+
+    /// The user cancelled. The cancel sequence goes out through `pump`.
+    pub fn cancel(&mut self) {
+        self.upload.cancel();
+    }
+
+    /// Moves the upload along. When `pending` (the session's unsent bytes) is
+    /// empty, fills it with what to send next. Returns `Some` once the upload
+    /// has ended and everything was sent, or `DRAIN_LIMIT` has passed since it
+    /// ended; `pending` is then cleared, and the session leaves upload mode.
+    pub fn pump(&mut self, now: Instant, pending: &mut Vec<u8>) -> Option<Finished> {
+        if pending.is_empty() {
+            self.upload.transmit(pending);
+        }
+        let state = self.upload.state();
+        if !state.is_final() {
+            return None;
+        }
+        let until = *self.drain_until.get_or_insert(now + DRAIN_LIMIT);
+        if !pending.is_empty() && now < until {
+            return None;
+        }
+        pending.clear();
+        let result = self.result(&state);
+        let mut output = self.upload.leftover();
+        output.extend_from_slice(result.line().as_bytes());
+        Some(Finished { output, result })
+    }
+
+    /// The state to report, if it changed and the last report is
+    /// `PROGRESS_INTERVAL` old. Final states are reported by `pump` instead.
+    pub fn progress(&mut self, now: Instant) -> Option<UploadState> {
+        let state = self.upload.state();
+        if state.is_final() || self.reported.as_ref() == Some(&state) {
+            return None;
+        }
+        if self
+            .reported_at
+            .is_some_and(|at| now.duration_since(at) < PROGRESS_INTERVAL)
+        {
+            return None;
+        }
+        self.reported = Some(state.clone());
+        self.reported_at = Some(now);
+        Some(state)
+    }
+
+    /// The connection ended in the middle of the upload.
+    pub fn lost(mut self) -> Finished {
+        let result = UploadResult {
+            ok: false,
+            message: "the connection was lost".to_string(),
+        };
+        let mut output = self.upload.leftover();
+        output.extend_from_slice(result.line().as_bytes());
+        Finished { output, result }
+    }
+
+    fn result(&self, state: &UploadState) -> UploadResult {
+        match state {
+            UploadState::Done {
+                sent_files,
+                skipped,
+            } => UploadResult {
+                ok: true,
+                message: self.done_message(*sent_files, *skipped),
+            },
+            UploadState::Failed(reason) => UploadResult {
+                ok: false,
+                message: reason.clone(),
+            },
+            _ => UploadResult {
+                ok: false,
+                message: CANCELLED.to_string(),
+            },
+        }
+    }
+
+    fn done_message(&self, sent: usize, skipped: usize) -> String {
+        let total: u64 = self.files.iter().map(|(_, size)| size).sum();
+        match self.files.as_slice() {
+            [(name, size)] if skipped == 0 => format!("uploaded {name} ({})", human_size(*size)),
+            [(name, _)] => format!("{name} was skipped by the receiver"),
+            files if skipped == 0 => {
+                format!("uploaded {} files ({})", files.len(), human_size(total))
+            }
+            files => format!(
+                "uploaded {sent} of {} files, {skipped} skipped by the receiver",
+                files.len()
+            ),
+        }
+    }
+}
+
+/// `512 B`, `1.2 KB`, `3.4 MB`, `1.0 GB` (powers of 1024).
+pub fn human_size(bytes: u64) -> String {
+    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
+    if bytes < 1024 {
+        return format!("{bytes} B");
+    }
+    let mut value = bytes as f64 / 1024.0;
+    let mut unit = 0;
+    while value >= 1024.0 && unit < UNITS.len() - 1 {
+        value /= 1024.0;
+        unit += 1;
+    }
+    format!("{value:.1} {}", UNITS[unit])
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer 2>&1 | grep -E "test result|FAILED|panicked|^error"; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: `transfer::active` 下 10 个测试通过，`transfer` 其余测试照旧通过。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core/src/transfer
git commit -m "feat(core): ActiveUpload, one upload inside a session loop"
```

---

### Task 3: 串口线程的传输模式

**Files:** Modify `crates/omnyssh-core/src/serial.rs`、`crates/omnyssh-gui/src/commands/serial.rs`

- 测试里的 `FakePort::fed()` 是一个由测试在会话运行中喂数据的假端口。
- GUI 这边 `route_output` 多了一个 `upload` 回调（`SerialOutput::Upload` 要有去处，否则 GUI 编译不过）。`serial_open` 暂时传一个空回调，Task 6 再接到事件上。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p3-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p3-test.patch && git apply /tmp/p3-test.patch`。

```diff
diff --git a/crates/omnyssh-core/src/serial.rs b/crates/omnyssh-core/src/serial.rs
--- a/crates/omnyssh-core/src/serial.rs
+++ b/crates/omnyssh-core/src/serial.rs
@@ -322,6 +322,8 @@
         /// progress — enough to defeat an attempt-count bound, not a time one.
         max_write_chunk: Option<usize>,
         written: Arc<Mutex<Vec<u8>>>,
+        /// Reads after the scripted ones, sent by the test while the session runs.
+        feed: Option<mpsc::Receiver<io::Result<Vec<u8>>>>,
     }
 
     impl FakePort {
@@ -342,8 +344,17 @@
                 writes: writes.into(),
                 max_write_chunk: None,
                 written: Arc::clone(&written),
+                feed: None,
             };
             (port, written)
+        }
+
+        /// A port whose reads come from the returned sender, one chunk per read.
+        fn fed() -> (Self, Arc<Mutex<Vec<u8>>>, Feed) {
+            let (mut port, written) = Self::new(vec![]);
+            let (tx, rx) = mpsc::channel();
+            port.feed = Some(rx);
+            (port, written, tx)
         }
 
         /// A port that only ever accepts `chunk` bytes per `write` call, no
@@ -354,6 +365,9 @@
             (port, written)
         }
     }
+
+    /// Sends a fed port its next read.
+    type Feed = mpsc::Sender<io::Result<Vec<u8>>>;
 
     impl Read for FakePort {
         fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
@@ -363,10 +377,17 @@
                     Ok(chunk.len())
                 }
                 Some(Err(e)) => Err(e),
-                None => {
-                    thread::sleep(Duration::from_millis(1));
-                    Err(io::Error::new(ErrorKind::TimedOut, "timed out"))
-                }
+                None => match self.feed.as_ref().and_then(|f| f.try_recv().ok()) {
+                    Some(Ok(chunk)) => {
+                        buf[..chunk.len()].copy_from_slice(&chunk);
+                        Ok(chunk.len())
+                    }
+                    Some(Err(e)) => Err(e),
+                    None => {
+                        thread::sleep(Duration::from_millis(1));
+                        Err(io::Error::new(ErrorKind::TimedOut, "timed out"))
+                    }
+                },
             }
         }
     }
@@ -531,6 +552,116 @@
         );
     }
 
+    use crate::transfer::{Protocol, UploadFile, UploadResult, UploadState};
+
+    const ACK: u8 = 0x06;
+
+    fn one_block_file() -> UploadFile {
+        UploadFile {
+            name: "a.bin".to_string(),
+            size: 128,
+            modified: None,
+            reader: Box::new(io::Cursor::new(vec![7u8; 128])),
+        }
+    }
+
+    /// Waits until the port has been written at least `len` bytes.
+    fn written_len(written: &Arc<Mutex<Vec<u8>>>, len: usize) -> Vec<u8> {
+        let deadline = Instant::now() + Duration::from_secs(2);
+        loop {
+            let w = written.lock().unwrap().clone();
+            if w.len() >= len {
+                return w;
+            }
+            assert!(Instant::now() < deadline, "only {} bytes written", w.len());
+            thread::sleep(Duration::from_millis(2));
+        }
+    }
+
+    /// Outputs up to and including the upload's `Finished`.
+    fn until_finished(rx: &mpsc::Receiver<SerialOutput>) -> (Vec<SerialOutput>, UploadResult) {
+        let mut seen = Vec::new();
+        loop {
+            match next(rx) {
+                SerialOutput::Upload(UploadEvent::Finished(result)) => return (seen, result),
+                other => seen.push(other),
+            }
+        }
+    }
+
+    #[test]
+    fn an_upload_takes_the_received_bytes_and_ends_with_a_line() {
+        let (port, written, feed) = FakePort::fed();
+        let (session, rx) = start(port);
+        let upload = ActiveUpload::start(Protocol::Xmodem, vec![one_block_file()], Instant::now());
+        session.upload(b"", upload);
+
+        feed.send(Ok(b"C".to_vec())).unwrap();
+        let block = written_len(&written, 133);
+        assert_eq!(block[0], 0x01, "a 128-byte block");
+        feed.send(Ok(vec![ACK])).unwrap();
+        assert_eq!(written_len(&written, 134)[133], 0x04, "EOT");
+        feed.send(Ok([&[ACK][..], b"=> "].concat())).unwrap();
+
+        let (seen, result) = until_finished(&rx);
+        assert!(result.ok, "{result:?}");
+        let data: Vec<&Vec<u8>> = seen
+            .iter()
+            .filter_map(|o| match o {
+                SerialOutput::Data(d) => Some(d),
+                _ => None,
+            })
+            .collect();
+        assert_eq!(
+            data,
+            [&b"=> \r\n[upload] uploaded a.bin (128 B)\r\n".to_vec()],
+            "protocol bytes never reach the terminal"
+        );
+        assert!(seen.contains(&SerialOutput::Upload(UploadEvent::Progress(
+            UploadState::Waiting
+        ))));
+
+        // Back to the terminal afterwards.
+        feed.send(Ok(b"ok".to_vec())).unwrap();
+        assert_eq!(next(&rx), SerialOutput::Data(b"ok".to_vec()));
+    }
+
+    #[test]
+    fn the_preamble_goes_out_and_a_cancel_ends_the_upload() {
+        let (port, written, _feed) = FakePort::fed();
+        let (session, rx) = start(port);
+        let upload = ActiveUpload::start(Protocol::Zmodem, vec![one_block_file()], Instant::now());
+        session.upload(b"rz -b\r", upload);
+        assert_eq!(written_len(&written, 6), b"rz -b\r");
+        session.cancel_upload();
+
+        let (seen, result) = until_finished(&rx);
+        assert_eq!(result.message, "cancelled");
+        assert!(seen.contains(&SerialOutput::Data(b"\r\n[upload] cancelled\r\n".to_vec())));
+        let w = written.lock().unwrap().clone();
+        assert!(
+            w[6..].starts_with(&[0x18; 10]),
+            "the cancel sequence: {w:?}"
+        );
+    }
+
+    #[test]
+    fn a_port_failing_mid_upload_reports_the_upload_lost() {
+        let (port, _written, feed) = FakePort::fed();
+        let (session, rx) = start(port);
+        let upload = ActiveUpload::start(Protocol::Xmodem, vec![one_block_file()], Instant::now());
+        session.upload(b"", upload);
+        feed.send(Err(io::Error::new(ErrorKind::BrokenPipe, "device removed")))
+            .unwrap();
+
+        let (_, result) = until_finished(&rx);
+        assert_eq!(result.message, "the connection was lost");
+        assert_eq!(
+            next(&rx),
+            SerialOutput::Closed(Some("device removed".to_string()))
+        );
+    }
+
     #[test]
     fn only_five_to_eight_data_bits_are_accepted() {
         assert!(data_bits(8).is_ok());
diff --git a/crates/omnyssh-gui/src/commands/serial.rs b/crates/omnyssh-gui/src/commands/serial.rs
--- a/crates/omnyssh-gui/src/commands/serial.rs
+++ b/crates/omnyssh-gui/src/commands/serial.rs
@@ -205,6 +205,7 @@
     use omnyssh_core::config::serial_devices::LineEnding;
     use omnyssh_core::serial::{FlowControl, Parity, StopBits};
     use omnyssh_core::session_log::{SessionLogger, TextLogger, NATIVE_NEWLINE};
+    use omnyssh_core::transfer::UploadState;
     use std::sync::{Arc, Mutex};
 
     fn device(name: &str, port: &str) -> SerialDevice {
@@ -267,6 +268,31 @@
     }
 
     #[test]
+    fn upload_events_go_to_their_own_route_not_the_log() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("COM3.log");
+        let logs = SessionLogs::default();
+        let logger = SessionLogger::Text(TextLogger::new(false, NATIVE_NEWLINE));
+        logs.start(4, &path, logger, "# COM3").unwrap();
+
+        let mut got = None;
+        route_output(
+            &logs,
+            4,
+            SerialOutput::Upload(UploadEvent::Progress(UploadState::Waiting)),
+            |_| panic!("not terminal output"),
+            |_| panic!("not an exit"),
+            |e| got = Some(e),
+        );
+        assert_eq!(got, Some(UploadEvent::Progress(UploadState::Waiting)));
+        logs.stop(4);
+        assert_eq!(
+            std::fs::read_to_string(&path).unwrap(),
+            format!("# COM3{NATIVE_NEWLINE}")
+        );
+    }
+
+    #[test]
     fn serial_output_reaches_the_log_and_a_failure_ends_it() {
         let dir = tempfile::tempdir().unwrap();
         let path = dir.path().join("COM3.log");
@@ -284,6 +310,7 @@
             SerialOutput::Data(b"boot ok\n".to_vec()),
             |b| shown = b,
             |_| {},
+            |_| {},
         );
         assert_eq!(shown, b"boot ok\n");
         let mut why = None;
@@ -293,6 +320,7 @@
             SerialOutput::Closed(Some("unplugged".to_string())),
             |_| {},
             |e| why = Some(e),
+            |_| {},
         );
         assert_eq!(why.as_deref(), Some("unplugged"));
         assert!(!logs.is_logging(4));
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test --workspace --no-run 2>&1 | grep -E "^error" | sort | uniq -c; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 编译失败：core 报 `cannot find type ActiveUpload/UploadEvent`、`SerialSession` 没有 `upload`/`cancel_upload`；GUI 报 `route_output` 多了一个参数（`this function takes 5 arguments but 6 arguments were supplied`）。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p3-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p3-impl.patch && git apply /tmp/p3-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/serial.rs b/crates/omnyssh-core/src/serial.rs
--- a/crates/omnyssh-core/src/serial.rs
+++ b/crates/omnyssh-core/src/serial.rs
@@ -13,6 +13,8 @@
 
 use anyhow::{bail, Context, Result};
 use serde::{Deserialize, Serialize};
+
+use crate::transfer::{ActiveUpload, UploadEvent};
 
 /// How long one read or one write attempt waits before the loop checks for
 /// new input again. Because it bounds writes too, a single stalled write
@@ -84,10 +86,22 @@
     /// The session ended; sent exactly once. `None` when the caller closed it,
     /// the reason when the port failed (adapter unplugged, driver error).
     Closed(Option<String>),
+    /// How the upload started with [`SerialSession::upload`] is going. While
+    /// one runs, received bytes go to it instead of `Data`; at the end the
+    /// device's last output and the result line come as `Data`, then
+    /// `Finished`.
+    Upload(UploadEvent),
 }
 
 enum Ctrl {
     Write(Vec<u8>),
+    /// Sends `preamble` and enters upload mode in one step, so no byte read in
+    /// between can miss the upload (`rz`'s first ZRINIT, say).
+    Upload {
+        preamble: Vec<u8>,
+        upload: ActiveUpload,
+    },
+    CancelUpload,
     Close,
 }
 
@@ -155,6 +169,20 @@
         let _ = self.ctrl.send(Ctrl::Write(data.to_vec()));
     }
 
+    /// Sends `preamble` (`rz -b\r` for ZMODEM, nothing for X/YMODEM) and runs
+    /// `upload` until it ends. The caller makes sure no other upload is running.
+    pub fn upload(&self, preamble: &[u8], upload: ActiveUpload) {
+        let _ = self.ctrl.send(Ctrl::Upload {
+            preamble: preamble.to_vec(),
+            upload,
+        });
+    }
+
+    /// Cancels the running upload, if any.
+    pub fn cancel_upload(&self) {
+        let _ = self.ctrl.send(Ctrl::CancelUpload);
+    }
+
     /// Asks the session to close the port. Non-blocking: the thread finishes
     /// asynchronously. `Drop` is what waits for it.
     pub fn close(&self) {
@@ -223,6 +251,9 @@
 /// one write of it, then — unless closing — wait up to `IO_TIMEOUT` for bytes.
 /// Runs until closed or the port fails; reports `Closed` exactly once.
 ///
+/// During an upload the received bytes go to it, and it adds to `pending`
+/// whenever `pending` is empty, so the same bounded writes carry it.
+///
 /// Writes never call `write_all`/`flush`: on a stalled line those can block for
 /// as long as the OS lets them (on Windows, `FlushFileBuffers` can wait
 /// forever), holding the port's exclusive lock. Instead each iteration makes
@@ -235,16 +266,41 @@
 ) {
     let mut buf = [0u8; 4096];
     let mut pending: Vec<u8> = Vec::new();
+    let mut upload: Option<ActiveUpload> = None;
     loop {
         let mut closing = false;
         loop {
             match ctrl.try_recv() {
                 Ok(Ctrl::Write(data)) => pending.extend_from_slice(&data),
+                Ok(Ctrl::Upload {
+                    preamble,
+                    upload: started,
+                }) => {
+                    pending.extend_from_slice(&preamble);
+                    upload = Some(started);
+                }
+                Ok(Ctrl::CancelUpload) => {
+                    if let Some(up) = upload.as_mut() {
+                        up.cancel();
+                    }
+                }
                 Ok(Ctrl::Close) | Err(TryRecvError::Disconnected) => {
                     closing = true;
                     break;
                 }
                 Err(TryRecvError::Empty) => break,
+            }
+        }
+
+        if let Some(up) = upload.as_mut() {
+            let now = Instant::now();
+            up.tick(now);
+            if let Some(end) = up.pump(now, &mut pending) {
+                upload = None;
+                on_output(SerialOutput::Data(end.output));
+                on_output(SerialOutput::Upload(UploadEvent::Finished(end.result)));
+            } else if let Some(state) = up.progress(now) {
+                on_output(SerialOutput::Upload(UploadEvent::Progress(state)));
             }
         }
 
@@ -259,6 +315,7 @@
                         ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                     ) => {}
                 Err(e) => {
+                    end_upload(upload, &mut on_output);
                     on_output(SerialOutput::Closed(Some(e.to_string())));
                     return;
                 }
@@ -286,20 +343,34 @@
         match port.read(&mut buf) {
             // Windows never returns this; POSIX does on hang-up.
             Ok(0) => {
+                end_upload(upload, &mut on_output);
                 on_output(SerialOutput::Closed(Some("port closed".to_string())));
                 return;
             }
-            Ok(n) => on_output(SerialOutput::Data(buf[..n].to_vec())),
+            Ok(n) => match upload.as_mut() {
+                Some(up) => up.receive(&buf[..n], Instant::now()),
+                None => on_output(SerialOutput::Data(buf[..n].to_vec())),
+            },
             Err(e)
                 if matches!(
                     e.kind(),
                     ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                 ) => {}
             Err(e) => {
+                end_upload(upload, &mut on_output);
                 on_output(SerialOutput::Closed(Some(e.to_string())));
                 return;
             }
         }
+    }
+}
+
+/// The port failed during an upload: report it lost before the session ends.
+fn end_upload(upload: Option<ActiveUpload>, on_output: &mut impl FnMut(SerialOutput)) {
+    if let Some(up) = upload {
+        let end = up.lost();
+        on_output(SerialOutput::Data(end.output));
+        on_output(SerialOutput::Upload(UploadEvent::Finished(end.result)));
     }
 }
 
diff --git a/crates/omnyssh-gui/src/commands/serial.rs b/crates/omnyssh-gui/src/commands/serial.rs
--- a/crates/omnyssh-gui/src/commands/serial.rs
+++ b/crates/omnyssh-gui/src/commands/serial.rs
@@ -8,6 +8,7 @@
 };
 use omnyssh_core::event::SessionId;
 use omnyssh_core::serial::{self, SerialOutput, SerialSession};
+use omnyssh_core::transfer::UploadEvent;
 use tauri::ipc::Channel;
 use tauri::State;
 
@@ -59,6 +60,8 @@
                 |error| {
                     let _ = on_exit.send(SerialExitDto { error });
                 },
+                // Nothing starts an upload yet.
+                |_| {},
             )
         })
     })
@@ -74,13 +77,16 @@
 }
 
 /// Pass one event of serial session `id` on: received bytes to its log, if one is
-/// being written, and to the tab; a failure ends the log, then reaches the tab.
+/// being written, and to the tab; a failure ends the log, then reaches the tab;
+/// upload events go out as `upload-*` events. During an upload no protocol bytes
+/// come as `Data`, so none reach the log; its result line does.
 fn route_output(
     logs: &SessionLogs,
     id: SessionId,
     out: SerialOutput,
     data: impl FnOnce(Vec<u8>),
     exit: impl FnOnce(String),
+    upload: impl FnOnce(UploadEvent),
 ) {
     match out {
         SerialOutput::Data(bytes) => {
@@ -93,6 +99,7 @@
         }
         // Closed by the user: the tab is already gone, and `serial_close` ends the log.
         SerialOutput::Closed(None) => {}
+        SerialOutput::Upload(event) => upload(event),
     }
 }
 
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib serial 2>&1 | grep -E "test result|FAILED|panicked"; cargo test -p omnyssh-gui serial 2>&1 | grep -E "test result|FAILED|panicked"; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: core 的 `serial` 测试全部通过，包括 `an_upload_takes_the_received_bytes_and_ends_with_a_line`、`the_preamble_goes_out_and_a_cancel_ends_the_upload`、`a_port_failing_mid_upload_reports_the_upload_lost`；GUI 的 `upload_events_go_to_their_own_route_not_the_log` 通过。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core/src/serial.rs crates/omnyssh-gui/src/commands/serial.rs
git commit -m "feat(core): uploads in the serial session thread"
```

---

### Task 4: SSH pump 的传输模式

**Files:** Modify `crates/omnyssh-core/src/ssh/pty.rs`、`crates/omnyssh-core/src/event.rs`、`crates/omnyssh/src/app/mod.rs`；Create `crates/omnyssh-core/tests/upload_pty.rs`

- 测试起一个进程内 SSH 服务端，它的 "shell" 按行读输入：收到 `rz -b` 时，如果有 lrzsz 就把通道接到真实的 `rz`，没有就回 `rz: command not found`。
- `a_zmodem_upload_through_a_terminal_reaches_rz` 需要 lrzsz，标了 `#[ignore]`；`cancelling_an_upload_the_shell_cannot_take_shows_its_reply` 不需要，平时就跑。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p4-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p4-test.patch && git apply /tmp/p4-test.patch`。

```diff
diff --git a/crates/omnyssh-core/tests/upload_pty.rs b/crates/omnyssh-core/tests/upload_pty.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-core/tests/upload_pty.rs
@@ -0,0 +1,344 @@
+//! Uploads through a terminal, end to end: `PtyManager` against an in-process SSH
+//! server whose "shell" reads lines and, given `rz -b`, runs the real `rz` on
+//! the channel — or answers `command not found` when there is none.
+
+#![cfg(unix)]
+
+use std::net::SocketAddr;
+use std::path::PathBuf;
+use std::process::Stdio;
+use std::sync::{Arc, Mutex, Once};
+use std::time::{Duration, Instant};
+
+use russh::keys::key::KeyPair;
+use russh::server::{self, Auth, Msg, Session};
+use russh::{Channel, ChannelId, CryptoVec};
+use tokio::io::{AsyncReadExt, AsyncWriteExt};
+use tokio::net::TcpListener;
+use tokio::sync::mpsc;
+
+use omnyssh_core::event::CoreEvent;
+use omnyssh_core::ssh::client::Host;
+use omnyssh_core::ssh::pty::PtyManager;
+use omnyssh_core::transfer::{
+    ActiveUpload, Protocol, UploadEvent, UploadFile, UploadResult, UploadState,
+};
+
+const PASSWORD: &str = "upload-test";
+
+/// Keeps trust-on-first-use off the real `~/.ssh`, and the local agent out.
+fn isolate_home() {
+    static ONCE: Once = Once::new();
+    ONCE.call_once(|| {
+        let home = tempfile::tempdir().expect("tempdir").keep();
+        std::env::set_var("HOME", &home);
+        std::env::remove_var("SSH_AUTH_SOCK");
+    });
+}
+
+/// `rz` from `LRZSZ_DIR`, or from `PATH`.
+fn rz() -> PathBuf {
+    std::env::var_os("LRZSZ_DIR")
+        .map(|dir| PathBuf::from(dir).join("rz"))
+        .unwrap_or_else(|| PathBuf::from("rz"))
+}
+
+// ---------------------------------------------------------------------------
+// SSH server
+// ---------------------------------------------------------------------------
+
+#[derive(Clone)]
+struct Server {
+    /// Where `rz` saves; `None` plays a shell without lrzsz.
+    rz_dir: Option<PathBuf>,
+    /// The line typed so far, while no `rz` runs.
+    line: Arc<Mutex<Vec<u8>>>,
+    /// Input for the running `rz`.
+    to_rz: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
+}
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
+    async fn channel_open_session(
+        &mut self,
+        _channel: Channel<Msg>,
+        _session: &mut Session,
+    ) -> Result<bool, Self::Error> {
+        Ok(true)
+    }
+
+    async fn shell_request(
+        &mut self,
+        channel: ChannelId,
+        session: &mut Session,
+    ) -> Result<(), Self::Error> {
+        session.data(channel, CryptoVec::from_slice(b"$ "));
+        Ok(())
+    }
+
+    async fn data(
+        &mut self,
+        channel: ChannelId,
+        data: &[u8],
+        session: &mut Session,
+    ) -> Result<(), Self::Error> {
+        if let Some(to_rz) = self.to_rz.lock().unwrap().as_ref() {
+            let _ = to_rz.send(data.to_vec());
+            return Ok(());
+        }
+        for &byte in data {
+            if byte != b'\r' {
+                self.line.lock().unwrap().push(byte);
+                continue;
+            }
+            let line = std::mem::take(&mut *self.line.lock().unwrap());
+            match (&self.rz_dir, line.as_slice()) {
+                (Some(dir), b"rz -b") => self.start_rz(dir, channel, session),
+                (None, b"rz -b") => session.data(
+                    channel,
+                    CryptoVec::from_slice(b"rz -b\r\nsh: rz: command not found\r\n$ "),
+                ),
+                _ => session.data(channel, CryptoVec::from_slice(b"\r\n$ ")),
+            }
+        }
+        Ok(())
+    }
+}
+
+impl Server {
+    /// Runs `rz -b` in `dir` with its stdin and stdout on the channel, then
+    /// prints a prompt when it exits, the way a shell would.
+    fn start_rz(&self, dir: &PathBuf, channel: ChannelId, session: &Session) {
+        let mut child = tokio::process::Command::new(rz())
+            .arg("-b")
+            .current_dir(dir)
+            .stdin(Stdio::piped())
+            .stdout(Stdio::piped())
+            .stderr(Stdio::null())
+            .kill_on_drop(true)
+            .spawn()
+            .expect("cannot run rz (is lrzsz installed? set LRZSZ_DIR)");
+        let mut stdin = child.stdin.take().unwrap();
+        let mut stdout = child.stdout.take().unwrap();
+        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
+        *self.to_rz.lock().unwrap() = Some(tx);
+        tokio::spawn(async move {
+            while let Some(chunk) = rx.recv().await {
+                if stdin.write_all(&chunk).await.is_err() {
+                    break;
+                }
+            }
+        });
+        let handle = session.handle();
+        let to_rz = Arc::clone(&self.to_rz);
+        tokio::spawn(async move {
+            let mut buf = vec![0u8; 16 * 1024];
+            loop {
+                match stdout.read(&mut buf).await {
+                    Ok(0) | Err(_) => break,
+                    Ok(n) => {
+                        let _ = handle.data(channel, CryptoVec::from_slice(&buf[..n])).await;
+                    }
+                }
+            }
+            let _ = child.wait().await;
+            *to_rz.lock().unwrap() = None;
+            let _ = handle.data(channel, CryptoVec::from_slice(b"$ ")).await;
+        });
+    }
+}
+
+async fn serve(rz_dir: Option<PathBuf>) -> SocketAddr {
+    let server = Server {
+        rz_dir,
+        line: Arc::default(),
+        to_rz: Arc::default(),
+    };
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
+            let _ = server::run_stream(Arc::clone(&config), socket, server.clone()).await;
+        }
+    });
+    addr
+}
+
+// ---------------------------------------------------------------------------
+// Client side
+// ---------------------------------------------------------------------------
+
+struct Terminal {
+    pty: PtyManager,
+    id: u64,
+    events: mpsc::Receiver<CoreEvent>,
+}
+
+impl Terminal {
+    async fn open(addr: SocketAddr) -> Self {
+        isolate_home();
+        let host = Host {
+            name: "box".to_string(),
+            hostname: addr.ip().to_string(),
+            port: addr.port(),
+            user: "me".to_string(),
+            password: Some(PASSWORD.to_string()),
+            ..Host::default()
+        };
+        let (tx, events) = mpsc::channel::<CoreEvent>(1024);
+        let mut pty = PtyManager::new();
+        let id = pty.open(&host, 80, 24, tx).expect("open");
+        let term = Self { pty, id, events };
+        assert!(term.screen_contains("$ ").await, "no shell");
+        term
+    }
+
+    fn screen(&self) -> String {
+        let parser = self.pty.parser_for(self.id).expect("session");
+        let text = parser.lock().unwrap().screen().contents();
+        text
+    }
+
+    async fn screen_contains(&self, text: &str) -> bool {
+        for _ in 0..200 {
+            if self.screen().contains(text) {
+                return true;
+            }
+            tokio::time::sleep(Duration::from_millis(50)).await;
+        }
+        false
+    }
+
+    /// Upload events until the end, which is returned last.
+    async fn until_finished(&mut self) -> (Vec<UploadState>, UploadResult) {
+        let mut progress = Vec::new();
+        loop {
+            let event = tokio::time::timeout(Duration::from_secs(30), self.events.recv())
+                .await
+                .expect("the upload never finished")
+                .expect("session gone");
+            match event {
+                CoreEvent::Upload(id, UploadEvent::Progress(state)) if id == self.id => {
+                    progress.push(state)
+                }
+                CoreEvent::Upload(id, UploadEvent::Finished(result)) if id == self.id => {
+                    return (progress, result)
+                }
+                _ => {}
+            }
+        }
+    }
+}
+
+fn file(name: &str, data: Vec<u8>) -> UploadFile {
+    UploadFile {
+        name: name.to_string(),
+        size: data.len() as u64,
+        modified: Some(1_700_000_000),
+        reader: Box::new(std::io::Cursor::new(data)),
+    }
+}
+
+/// Random-looking bytes with every value, so escaping is exercised.
+fn bytes(len: usize) -> Vec<u8> {
+    let mut x: u32 = 0x9e37_79b9;
+    (0..len)
+        .map(|_| {
+            x ^= x << 13;
+            x ^= x >> 17;
+            x ^= x << 5;
+            x as u8
+        })
+        .collect()
+}
+
+// ---------------------------------------------------------------------------
+// Tests
+// ---------------------------------------------------------------------------
+
+#[tokio::test]
+#[ignore = "needs lrzsz"]
+async fn a_zmodem_upload_through_a_terminal_reaches_rz() {
+    let dir = tempfile::tempdir().unwrap();
+    let addr = serve(Some(dir.path().to_path_buf())).await;
+    let mut term = Terminal::open(addr).await;
+
+    let data = bytes(300 * 1024);
+    let upload = ActiveUpload::start(
+        Protocol::Zmodem,
+        vec![
+            file("big.bin", data.clone()),
+            file("small.txt", b"hi\n".to_vec()),
+        ],
+        Instant::now(),
+    );
+    term.pty.upload(term.id, b"rz -b\r", upload);
+
+    let (progress, result) = term.until_finished().await;
+    assert!(result.ok, "{result:?}");
+    assert_eq!(result.message, "uploaded 2 files (300.0 KB)");
+    assert!(progress
+        .iter()
+        .any(|s| matches!(s, UploadState::Sending { .. })));
+    assert_eq!(std::fs::read(dir.path().join("big.bin")).unwrap(), data);
+    assert_eq!(
+        std::fs::read(dir.path().join("small.txt")).unwrap(),
+        b"hi\n"
+    );
+
+    assert!(term.screen_contains("[upload] uploaded 2 files").await);
+    // No protocol bytes on the screen: the frames start with `**` + ZDLE.
+    assert!(!term.screen().contains("**"), "{}", term.screen());
+    // The terminal is the shell's again: a typed line gets a new prompt.
+    let prompts = term.screen().matches("$ ").count();
+    term.pty.write(term.id, b"ls\r").unwrap();
+    let mut back = false;
+    for _ in 0..100 {
+        if term.screen().matches("$ ").count() > prompts {
+            back = true;
+            break;
+        }
+        tokio::time::sleep(Duration::from_millis(50)).await;
+    }
+    assert!(back, "{}", term.screen());
+}
+
+#[tokio::test]
+async fn cancelling_an_upload_the_shell_cannot_take_shows_its_reply() {
+    let addr = serve(None).await;
+    let mut term = Terminal::open(addr).await;
+
+    let upload = ActiveUpload::start(
+        Protocol::Zmodem,
+        vec![file("a.bin", vec![1, 2, 3])],
+        Instant::now(),
+    );
+    term.pty.upload(term.id, b"rz -b\r", upload);
+    // The shell's answer is held back while the upload waits for rz.
+    tokio::time::sleep(Duration::from_millis(500)).await;
+    assert!(!term.screen().contains("command not found"));
+
+    term.pty.cancel_upload(term.id);
+    let (_, result) = term.until_finished().await;
+    assert_eq!(result.message, "cancelled");
+    assert!(term.screen_contains("command not found").await);
+    assert!(term.screen_contains("[upload] cancelled").await);
+}
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-core --test upload_pty --no-run 2>&1 | grep -E "^error" | sort | uniq -c; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 编译失败（只有 `upload_pty` 这个测试目标）：`PtyManager` 没有 `upload`/`cancel_upload`，`CoreEvent` 没有 `Upload`。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p4-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p4-impl.patch && git apply /tmp/p4-impl.patch`。

```diff
diff --git a/crates/omnyssh-core/src/event.rs b/crates/omnyssh-core/src/event.rs
--- a/crates/omnyssh-core/src/event.rs
+++ b/crates/omnyssh-core/src/event.rs
@@ -12,6 +12,7 @@
 use crate::ssh::key_setup::KeySetupStep;
 use crate::ssh::sftp::FileEntry;
 use crate::ssh::tunnel::TunnelStatus;
+use crate::transfer::UploadEvent;
 
 /// Placeholder type aliases for future stages.
 /// `HostId` is the host's `name` field — stable, human-readable key.
@@ -123,6 +124,8 @@
     PtyOutput(SessionId),
     /// The PTY child process exited (reader thread reached EOF or I/O error).
     PtyExited(SessionId),
+    /// How an upload started with `PtyManager::upload` is going (GUI only).
+    Upload(SessionId, UploadEvent),
 
     // -----------------------------------------------------------------------
     // Smart Server Context — Discovery events
diff --git a/crates/omnyssh-core/src/ssh/pty.rs b/crates/omnyssh-core/src/ssh/pty.rs
--- a/crates/omnyssh-core/src/ssh/pty.rs
+++ b/crates/omnyssh-core/src/ssh/pty.rs
@@ -12,11 +12,13 @@
 //! application layer.
 
 use std::sync::{Arc, Mutex};
+use std::time::{Duration, Instant};
 
 use anyhow::{Context, Result};
 use async_trait::async_trait;
 use russh::ChannelMsg;
 use tokio::sync::mpsc;
+use tokio::time::MissedTickBehavior;
 
 use crate::event::CoreEvent;
 use crate::ssh::client::Host;
@@ -26,6 +28,7 @@
 use crate::ssh::session::{
     connect_for_shell, forwards_agent, passphrase_required, Passwords, SshConnection,
 };
+use crate::transfer::{ActiveUpload, UploadEvent, UploadResult};
 
 /// Stable numeric identifier for a PTY session (mirrors [`crate::event::SessionId`]).
 pub type SessionId = u64;
@@ -39,7 +42,17 @@
     /// Keystrokes or pasted bytes for the remote shell.
     Input(Vec<u8>),
     /// Window resize; forwarded to the server as `window_change`.
-    Resize { cols: u16, rows: u16 },
+    Resize {
+        cols: u16,
+        rows: u16,
+    },
+    /// Sends `preamble` and enters upload mode in the same step, so no output
+    /// read in between can miss the upload (`rz`'s first ZRINIT, say).
+    Upload {
+        preamble: Vec<u8>,
+        upload: ActiveUpload,
+    },
+    CancelUpload,
     /// Request the task to end and close the connection.
     Close,
 }
@@ -209,6 +222,7 @@
         size: (cols, rows),
         asked: false,
         closed: false,
+        refused_uploads: 0,
     };
     // Asked once: the connection that takes agent channels and the request that
     // invites them must agree, even if the agent comes or goes during the login.
@@ -227,7 +241,7 @@
     if prompt.asked {
         prompt.flush();
     }
-    let ((cols, rows), closed) = (prompt.size, prompt.closed);
+    let ((cols, rows), closed, refused) = (prompt.size, prompt.closed, prompt.refused_uploads);
     if closed && connected.is_ok() {
         // Closed at the prompt: no shell for a tab that is gone.
         let _ = tx.send(CoreEvent::PtyExited(id)).await;
@@ -256,23 +270,39 @@
         }
     };
 
+    // Uploads sent while the password prompt had the keyboard cannot run.
+    for _ in 0..refused {
+        let result = UploadResult {
+            ok: false,
+            message: "the terminal was still logging in".to_string(),
+        };
+        let _ = tx
+            .send(CoreEvent::Upload(id, UploadEvent::Finished(result)))
+            .await;
+    }
+
     // Phase C: pump loop (official russh interactive idiom). `wait` is the only
     // &mut method, so a single owning task can select over it and the control
     // receiver without aliasing.
+    let out = Output {
+        id,
+        parser: &parser,
+        tx: &tx,
+        raw_output: raw_output.as_ref(),
+    };
+    let mut upload: Option<ActiveUpload> = None;
+    let mut ticker = tokio::time::interval(UPLOAD_TICK);
+    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
     loop {
         tokio::select! {
             msg = channel.wait() => match msg {
                 // stdout and remote stderr both render in a real terminal.
                 Some(ChannelMsg::Data { data })
-                | Some(ChannelMsg::ExtendedData { data, .. }) => {
-                    feed_parser(&parser, &data);
-                    let _ = tx.send(CoreEvent::PtyOutput(id)).await;
-                    // Additive GUI tap (tech-gui.md §3.6): mirror the raw bytes to
-                    // the frontend channel. Absent for the TUI → no behaviour change.
-                    if let Some(raw) = &raw_output {
-                        let _ = raw.send((id, data.to_vec())).await;
-                    }
-                }
+                | Some(ChannelMsg::ExtendedData { data, .. }) => match upload.as_mut() {
+                    // An upload owns the stream: nothing of it reaches the screen.
+                    Some(up) => up.receive(&data, Instant::now()),
+                    None => out.show(&data).await,
+                },
                 Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                 _ => {} // ExitStatus etc.: ignore, wait for Close.
             },
@@ -283,14 +313,97 @@
                 Some(Ctrl::Resize { cols, rows }) => {
                     let _ = channel.window_change(cols as u32, rows as u32, 0, 0).await;
                 }
+                Some(Ctrl::Upload { preamble, upload: started }) => {
+                    let _ = channel.data(&preamble[..]).await;
+                    upload = Some(started);
+                }
+                Some(Ctrl::CancelUpload) => {
+                    if let Some(up) = upload.as_mut() {
+                        up.cancel();
+                    }
+                }
                 Some(Ctrl::Close) | None => break,
             },
-        }
+            _ = ticker.tick(), if upload.is_some() => {
+                if let Some(up) = upload.as_mut() {
+                    up.tick(Instant::now());
+                }
+            }
+        }
+        if let Some(up) = upload.as_mut() {
+            if let Some(end) = pump_upload(up, &channel, &out).await {
+                upload = None;
+                out.show(&end.output).await;
+                let _ = tx
+                    .send(CoreEvent::Upload(id, UploadEvent::Finished(end.result)))
+                    .await;
+            }
+        }
+    }
+    if let Some(up) = upload {
+        let end = up.lost();
+        out.show(&end.output).await;
+        let _ = tx
+            .send(CoreEvent::Upload(id, UploadEvent::Finished(end.result)))
+            .await;
     }
 
     let _ = channel.eof().await;
     let _ = tx.send(CoreEvent::PtyExited(id)).await;
     // _handle drops here → russh closes the TCP connection.
+}
+
+/// How often a running upload's timers are checked when nothing arrives.
+const UPLOAD_TICK: Duration = Duration::from_millis(100);
+
+/// Where a session's screen output goes: the parser, the render nudge, and the
+/// GUI's raw tap.
+struct Output<'a> {
+    id: SessionId,
+    parser: &'a Arc<Mutex<vt100::Parser>>,
+    tx: &'a mpsc::Sender<CoreEvent>,
+    raw_output: Option<&'a mpsc::Sender<(SessionId, Vec<u8>)>>,
+}
+
+impl Output<'_> {
+    async fn show(&self, data: &[u8]) {
+        feed_parser(self.parser, data);
+        let _ = self.tx.send(CoreEvent::PtyOutput(self.id)).await;
+        // Additive GUI tap (tech-gui.md §3.6): mirror the raw bytes to the
+        // frontend channel. Absent for the TUI → no behaviour change.
+        if let Some(raw) = self.raw_output {
+            let _ = raw.send((self.id, data.to_vec())).await;
+        }
+    }
+}
+
+/// Sends what the upload has to send, until it waits for the device or ends.
+/// Each chunk is awaited whole, so russh's window paces it; the protocols stop
+/// on their own once their window is out, so this does not run away. Reports
+/// progress on the way; returns the end, if it came.
+async fn pump_upload(
+    up: &mut ActiveUpload,
+    channel: &russh::Channel<russh::client::Msg>,
+    out: &Output<'_>,
+) -> Option<crate::transfer::Finished> {
+    let mut pending = Vec::new();
+    loop {
+        let now = Instant::now();
+        if let Some(end) = up.pump(now, &mut pending) {
+            return Some(end);
+        }
+        if let Some(state) = up.progress(now) {
+            let _ = out
+                .tx
+                .send(CoreEvent::Upload(out.id, UploadEvent::Progress(state)))
+                .await;
+        }
+        if pending.is_empty() {
+            return None;
+        }
+        let _ = channel.data(&pending[..]).await;
+        pending.clear();
+    }
 }
 
 /// Asks for the login password inside the tab, the way ssh(1) does: the prompt
@@ -308,6 +421,8 @@
     asked: bool,
     /// The tab was closed at the prompt.
     closed: bool,
+    /// Uploads that arrived before the shell was there, to report as failed.
+    refused_uploads: usize,
 }
 
 impl InlinePrompt<'_> {
@@ -316,7 +431,8 @@
     fn flush(&mut self) {
         while let Ok(ctrl) = self.ctrl_rx.try_recv() {
             match ctrl {
-                Ctrl::Input(_) => {}
+                Ctrl::Input(_) | Ctrl::CancelUpload => {}
+                Ctrl::Upload { .. } => self.refused_uploads += 1,
                 Ctrl::Resize { cols, rows } => self.size = (cols, rows),
                 Ctrl::Close => self.closed = true,
             }
@@ -358,6 +474,8 @@
                     }
                 }
                 Some(Ctrl::Resize { cols, rows }) => self.size = (cols, rows),
+                Some(Ctrl::Upload { .. }) => self.refused_uploads += 1,
+                Some(Ctrl::CancelUpload) => {}
                 Some(Ctrl::Close) | None => {
                     self.closed = true;
                     return Err(NoAnswer::Cancelled);
@@ -537,6 +655,25 @@
         Ok(())
     }
 
+    /// Sends `preamble` (`rz -b\r` for ZMODEM) to session `id` and runs
+    /// `upload` over it until it ends, reported as [`CoreEvent::Upload`]. The
+    /// caller makes sure no other upload is running there. Unknown id: no-op.
+    pub fn upload(&mut self, id: SessionId, preamble: &[u8], upload: ActiveUpload) {
+        if let Some(s) = self.sessions.iter().find(|s| s.id == id) {
+            let _ = s.ctrl_tx.send(Ctrl::Upload {
+                preamble: preamble.to_vec(),
+                upload,
+            });
+        }
+    }
+
+    /// Cancels the upload running in session `id`, if any.
+    pub fn cancel_upload(&mut self, id: SessionId) {
+        if let Some(s) = self.sessions.iter().find(|s| s.id == id) {
+            let _ = s.ctrl_tx.send(Ctrl::CancelUpload);
+        }
+    }
+
     /// Forwards a resize to the session identified by `id`. No-op if not found.
     ///
     /// The vt100 parser is resized by the app layer; the task only relays
diff --git a/crates/omnyssh/src/app/mod.rs b/crates/omnyssh/src/app/mod.rs
--- a/crates/omnyssh/src/app/mod.rs
+++ b/crates/omnyssh/src/app/mod.rs
@@ -965,6 +965,8 @@
                 }
             }
 
+            // The TUI starts no uploads.
+            CoreEvent::Upload(..) => {}
             CoreEvent::Error(msg) => {
                 self.view.status_message = Some(msg);
             }
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --test upload_pty 2>&1 | grep -E "^test |test result"; LRZSZ_DIR=/tmp/lrzsz/usr/bin cargo test -p omnyssh-core --test upload_pty -- --ignored 2>&1 | grep -E "^test |test result"; cargo build -p omnyssh 2>&1 | tail -1; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

（本机 PATH 里有 lrzsz 时去掉 `LRZSZ_DIR=…`。）

Expected: 不加 `--ignored` 时 1 个通过、1 个 ignored；加 `--ignored` 时 `a_zmodem_upload_through_a_terminal_reaches_rz` 通过（不到 1 秒）；TUI 构建成功。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core/src crates/omnyssh-core/tests/upload_pty.rs crates/omnyssh/src/app/mod.rs
git commit -m "feat(core): uploads in the SSH terminal pump"
```

---

### Task 5: GUI 的上传逻辑（不含 Tauri）

**Files:** Create `crates/omnyssh-gui/src/upload.rs`；Modify `crates/omnyssh-gui/src/events.rs`、`crates/omnyssh-gui/src/main.rs`（`mod upload;`）

这一步的类型和函数要到 Task 6 才有人用，编译会有一批 dead_code 警告，属于正常；事件也要到 Task 6 才注册，所以 bindings 不变。

- [ ] **Step 1: 先加测试（新文件只有测试模块，`main.rs` 先声明模块）**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p5-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p5-test.patch && git apply /tmp/p5-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/src/main.rs b/crates/omnyssh-gui/src/main.rs
--- a/crates/omnyssh-gui/src/main.rs
+++ b/crates/omnyssh-gui/src/main.rs
@@ -14,6 +14,7 @@
 mod session_log;
 mod state;
 mod tray;
+mod upload;
 
 use commands::auth::{answer_host_key, answer_password, unlock_identity};
 use commands::hosts::{
diff --git a/crates/omnyssh-gui/src/upload.rs b/crates/omnyssh-gui/src/upload.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/src/upload.rs
@@ -0,0 +1,178 @@
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn file(name: &str, size: u64) -> Dropped {
+        Dropped {
+            path: format!("/tmp/{name}"),
+            name: name.to_string(),
+            size,
+            is_dir: false,
+        }
+    }
+
+    #[test]
+    fn a_drop_is_checked_before_anything_is_sent() {
+        let one = [file("a.bin", 10)];
+        let two = [file("a.bin", 10), file("b.bin", 10)];
+        for method in [Method::Zmodem, Method::Ymodem, Method::Xmodem, Method::Sftp] {
+            assert_eq!(check(method, &one), Ok(()));
+            assert_eq!(check(method, &[]), Err("nothing to upload".to_string()));
+        }
+        assert_eq!(check(Method::Ymodem, &two), Ok(()));
+        assert_eq!(
+            check(Method::Xmodem, &two),
+            Err("XMODEM sends one file at a time".to_string())
+        );
+        let mut dir = file("photos", 0);
+        dir.is_dir = true;
+        assert_eq!(
+            check(Method::Sftp, &[file("a", 1), dir]),
+            Err("folders cannot be uploaded: photos".to_string())
+        );
+    }
+
+    #[test]
+    fn zmodem_limits_size_and_zmodem_and_ymodem_limit_names() {
+        let big = [file("disk.img", 5 << 30)];
+        assert_eq!(
+            check(Method::Zmodem, &big),
+            Err("disk.img is larger than ZMODEM's 4 GiB limit".to_string())
+        );
+        assert_eq!(check(Method::Sftp, &big), Ok(()));
+        let long = [file(&"x".repeat(257), 1)];
+        assert!(check(Method::Zmodem, &long).is_err());
+        assert!(check(Method::Ymodem, &long).is_err());
+        assert_eq!(check(Method::Xmodem, &long), Ok(()), "XMODEM sends no name");
+    }
+
+    #[test]
+    fn inspect_reads_names_sizes_and_folders() {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("a.bin");
+        std::fs::write(&path, [0u8; 5]).unwrap();
+        let paths = [
+            path.to_string_lossy().into_owned(),
+            dir.path().to_string_lossy().into_owned(),
+        ];
+        let found = inspect(&paths).unwrap();
+        assert_eq!((found[0].name.as_str(), found[0].size), ("a.bin", 5));
+        assert!(!found[0].is_dir);
+        assert!(found[1].is_dir);
+        let missing = dir.path().join("gone").to_string_lossy().into_owned();
+        assert!(inspect(&[missing]).unwrap_err().starts_with("cannot read"));
+    }
+
+    #[test]
+    fn sftp_targets_resolve_against_the_home_folder() {
+        assert_eq!(sftp_target(None, "a.bin"), "a.bin");
+        assert_eq!(sftp_target(Some("~"), "a.bin"), "a.bin");
+        assert_eq!(sftp_target(Some(" ~/ "), "a.bin"), "a.bin");
+        assert_eq!(sftp_target(Some("~/drop/"), "a.bin"), "drop/a.bin");
+        assert_eq!(sftp_target(Some("/srv/in"), "a.bin"), "/srv/in/a.bin");
+        assert_eq!(sftp_target(Some("/"), "a.bin"), "/a.bin");
+        assert_eq!(sftp_target(Some("uploads"), "a.bin"), "uploads/a.bin");
+    }
+
+    #[test]
+    fn progress_events_mirror_the_state() {
+        let waiting = progress_event(7, &UploadState::Waiting).unwrap();
+        assert!(waiting.waiting);
+        let sending = progress_event(
+            7,
+            &UploadState::Sending {
+                file_index: 1,
+                file_count: 3,
+                file_name: "b".to_string(),
+                sent: 10,
+                total: 20,
+            },
+        )
+        .unwrap();
+        assert_eq!(
+            (sending.file_index, sending.file_count, sending.sent),
+            (1, 3, 10)
+        );
+        assert!(progress_event(7, &UploadState::Cancelled).is_none());
+    }
+
+    fn sftp_files() -> Vec<SftpFile> {
+        ["a", "b"]
+            .iter()
+            .map(|n| SftpFile {
+                local: format!("/l/{n}"),
+                remote: n.to_string(),
+                name: n.to_string(),
+                size: 1024,
+            })
+            .collect()
+    }
+
+    /// Where an upload command sends the file.
+    fn remote(step: Result<SftpCommand, UploadResult>) -> String {
+        match step {
+            Ok(SftpCommand::Upload { remote, .. }) => remote,
+            _ => panic!("expected the next upload"),
+        }
+    }
+
+    fn ended(step: Result<SftpCommand, UploadResult>) -> UploadResult {
+        match step {
+            Err(end) => end,
+            Ok(_) => panic!("expected the end"),
+        }
+    }
+
+    #[test]
+    fn sftp_files_go_one_after_another() {
+        let mut up = SftpUpload::new(sftp_files());
+        assert_eq!(remote(Ok(up.command())), "a");
+        assert_eq!(up.progress(4, 5, 1024).file_name, "a");
+        assert_eq!(remote(up.done(Ok(()))), "b");
+        assert_eq!(up.progress(4, 5, 1024).file_index, 1);
+        assert_eq!(
+            ended(up.done(Ok(()))),
+            UploadResult {
+                ok: true,
+                message: "uploaded 2 files (2.0 KB)".to_string()
+            }
+        );
+    }
+
+    #[test]
+    fn an_sftp_failure_or_cancel_stops_the_rest() {
+        let mut up = SftpUpload::new(sftp_files());
+        let end = ended(up.done(Err("permission denied".to_string())));
+        assert_eq!(end.message, "a: permission denied");
+        assert!(!end.ok);
+
+        let mut up = SftpUpload::new(sftp_files());
+        up.cancelled = true;
+        assert_eq!(ended(up.done(Ok(()))).message, "cancelled");
+    }
+
+    #[test]
+    fn one_upload_per_session() {
+        let uploads = Uploads::default();
+        assert_eq!(uploads.begin(1), Ok(()));
+        assert_eq!(
+            uploads.begin(1),
+            Err("an upload is already running here".to_string())
+        );
+        assert_eq!(uploads.begin(2), Ok(()));
+        uploads.end(1);
+        assert_eq!(uploads.begin(1), Ok(()));
+        uploads.session_closed(1);
+        assert_eq!(uploads.begin(1), Ok(()), "a closed session is free again");
+    }
+
+    #[test]
+    fn a_terminal_forgets_its_host_when_it_closes() {
+        let uploads = Uploads::default();
+        uploads.terminal_opened(3, "web");
+        assert_eq!(uploads.terminal_host(3).as_deref(), Some("web"));
+        uploads.session_closed(3);
+        assert_eq!(uploads.terminal_host(3), None);
+        assert!(uploads.sftp_lost(3).is_none());
+    }
+}
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-gui --no-run 2>&1 | grep -E "^error" | sort | uniq -c; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 编译失败：`upload` 模块里还只有测试，报找不到 `check`、`inspect`、`Dropped`、`SftpFile`、`UploadResult` 等。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p5-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p5-impl.patch && git apply /tmp/p5-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/src/events.rs b/crates/omnyssh-gui/src/events.rs
--- a/crates/omnyssh-gui/src/events.rs
+++ b/crates/omnyssh-gui/src/events.rs
@@ -239,6 +239,32 @@
     }
 }
 
+/// How a drag-and-drop upload in a terminal or serial tab is going. `waiting`
+/// while the receiver has not started yet (the other fields are then empty);
+/// `fileIndex` counts from 0. At most every 100 ms per session.
+#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
+#[serde(rename_all = "camelCase")]
+pub struct UploadProgress {
+    pub session_id: u64,
+    pub waiting: bool,
+    pub file_index: u32,
+    pub file_count: u32,
+    pub file_name: String,
+    pub sent: u64,
+    pub total: u64,
+}
+
+/// A drag-and-drop upload ended. `message` is what the terminal's result line
+/// says: `uploaded foo.bin (1.2 MB)`, `cancelled`, or why it failed. Ends every
+/// upload a command started, including one whose session went away.
+#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
+#[serde(rename_all = "camelCase")]
+pub struct UploadFinished {
+    pub session_id: u64,
+    pub ok: bool,
+    pub message: String,
+}
+
 /// A background error surfaced to the user.
 #[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
 pub struct Error {
diff --git a/crates/omnyssh-gui/src/upload.rs b/crates/omnyssh-gui/src/upload.rs
--- a/crates/omnyssh-gui/src/upload.rs
+++ b/crates/omnyssh-gui/src/upload.rs
@@ -1,3 +1,374 @@
+//! Drag-and-drop uploads, the parts that need no Tauri: the checks a drop goes
+//! through before anything is sent, where an SFTP upload lands, and the
+//! bookkeeping of which sessions are uploading. The protocols themselves run
+//! in the core's session loops; SFTP uploads run one file at a time on a
+//! connection kept per terminal.
+
+use std::collections::{HashMap, HashSet};
+use std::path::Path;
+use std::sync::Mutex;
+
+use omnyssh_core::event::SessionId;
+use omnyssh_core::ssh::sftp::{SftpCommand, SftpManager};
+use omnyssh_core::transfer::{human_size, UploadResult, UploadState};
+
+use crate::events::UploadProgress;
+
+/// Typed into the shell to start a ZMODEM upload: `-b` for binary. Not `-e`:
+/// the ZMODEM sender cannot escape every control character.
+pub const RZ_COMMAND: &[u8] = b"rz -b\r";
+
+/// The longest file name ZMODEM can carry, and the limit used for YMODEM too.
+const MAX_NAME: usize = 256;
+
+/// ZMODEM's file offsets are 32 bits.
+const ZMODEM_MAX_SIZE: u64 = u32::MAX as u64;
+
+/// How a drop is sent.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum Method {
+    Zmodem,
+    Ymodem,
+    Xmodem,
+    Sftp,
+}
+
+/// What the checks need to know about a dropped path.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct Dropped {
+    pub path: String,
+    pub name: String,
+    pub size: u64,
+    pub is_dir: bool,
+}
+
+/// Looks the dropped paths up on disk.
+pub fn inspect(paths: &[String]) -> Result<Vec<Dropped>, String> {
+    paths
+        .iter()
+        .map(|path| {
+            let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {path}: {e}"))?;
+            let name = Path::new(path)
+                .file_name()
+                .map(|n| n.to_string_lossy().into_owned())
+                .unwrap_or_else(|| path.clone());
+            Ok(Dropped {
+                path: path.clone(),
+                name,
+                size: meta.len(),
+                is_dir: meta.is_dir(),
+            })
+        })
+        .collect()
+}
+
+/// Whether `dropped` can go by `method`. Run before anything is sent, so a
+/// refused drop never starts `rz` or reaches the device.
+pub fn check(method: Method, dropped: &[Dropped]) -> Result<(), String> {
+    if dropped.is_empty() {
+        return Err("nothing to upload".to_string());
+    }
+    if let Some(dir) = dropped.iter().find(|d| d.is_dir) {
+        return Err(format!("folders cannot be uploaded: {}", dir.name));
+    }
+    if method == Method::Xmodem && dropped.len() > 1 {
+        return Err("XMODEM sends one file at a time".to_string());
+    }
+    for d in dropped {
+        if method == Method::Zmodem && d.size > ZMODEM_MAX_SIZE {
+            return Err(format!("{} is larger than ZMODEM's 4 GiB limit", d.name));
+        }
+        if matches!(method, Method::Zmodem | Method::Ymodem) && d.name.len() > MAX_NAME {
+            return Err(format!(
+                "{} has a name longer than {MAX_NAME} bytes",
+                d.name
+            ));
+        }
+    }
+    Ok(())
+}
+
+/// Where an SFTP upload of `name` lands. `dir` is the host's upload folder:
+/// unset or `~` is the home folder, `~/x` a folder in it, anything else as
+/// written. SFTP resolves relative paths against the home folder, so `~`
+/// becomes a relative path rather than being expanded here.
+pub fn sftp_target(dir: Option<&str>, name: &str) -> String {
+    let dir = dir.map(str::trim).unwrap_or("");
+    let dir = match dir {
+        "~" => "",
+        d => d.strip_prefix("~/").unwrap_or(d),
+    };
+    match dir.trim_end_matches('/') {
+        "" if dir.starts_with('/') => format!("/{name}"),
+        "" => name.to_string(),
+        dir => format!("{dir}/{name}"),
+    }
+}
+
+/// The progress event for `state`; `None` for a final state.
+pub fn progress_event(session_id: SessionId, state: &UploadState) -> Option<UploadProgress> {
+    match state {
+        UploadState::Waiting => Some(UploadProgress {
+            session_id,
+            waiting: true,
+            file_index: 0,
+            file_count: 0,
+            file_name: String::new(),
+            sent: 0,
+            total: 0,
+        }),
+        UploadState::Sending {
+            file_index,
+            file_count,
+            file_name,
+            sent,
+            total,
+        } => Some(UploadProgress {
+            session_id,
+            waiting: false,
+            file_index: *file_index as u32,
+            file_count: *file_count as u32,
+            file_name: file_name.clone(),
+            sent: *sent,
+            total: *total,
+        }),
+        _ => None,
+    }
+}
+
+/// One file of an SFTP upload.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct SftpFile {
+    pub local: String,
+    pub remote: String,
+    pub name: String,
+    pub size: u64,
+}
+
+/// An SFTP upload: files go one at a time, the next once the last is done, so
+/// a failure or a cancel stops the rest.
+#[derive(Debug)]
+pub struct SftpUpload {
+    files: Vec<SftpFile>,
+    /// The file being sent.
+    current: usize,
+    cancelled: bool,
+}
+
+impl SftpUpload {
+    pub fn new(files: Vec<SftpFile>) -> Self {
+        Self {
+            files,
+            current: 0,
+            cancelled: false,
+        }
+    }
+
+    fn command(&self) -> SftpCommand {
+        let file = &self.files[self.current];
+        SftpCommand::Upload {
+            local: file.local.clone(),
+            remote: file.remote.clone(),
+            transfer_id: self.current as u64,
+        }
+    }
+
+    fn progress(&self, session_id: SessionId, done: u64, total: u64) -> UploadProgress {
+        UploadProgress {
+            session_id,
+            waiting: false,
+            file_index: self.current as u32,
+            file_count: self.files.len() as u32,
+            file_name: self.files[self.current].name.clone(),
+            sent: done,
+            total,
+        }
+    }
+
+    /// The current file is done: the next command to send, or the end.
+    fn done(&mut self, result: Result<(), String>) -> Result<SftpCommand, UploadResult> {
+        if let Err(e) = result {
+            return Err(UploadResult {
+                ok: false,
+                message: format!("{}: {e}", self.files[self.current].name),
+            });
+        }
+        if self.cancelled {
+            return Err(UploadResult {
+                ok: false,
+                message: "cancelled".to_string(),
+            });
+        }
+        self.current += 1;
+        if self.current < self.files.len() {
+            return Ok(self.command());
+        }
+        let message = match self.files.as_slice() {
+            [file] => format!("uploaded {} ({})", file.name, human_size(file.size)),
+            files => {
+                let total = files.iter().map(|f| f.size).sum();
+                format!("uploaded {} files ({})", files.len(), human_size(total))
+            }
+        };
+        Err(UploadResult { ok: true, message })
+    }
+}
+
+/// A terminal's SFTP connection for uploads, and the upload on it, if any.
+struct SftpConnection {
+    manager: SftpManager,
+    upload: Option<SftpUpload>,
+}
+
+/// Which sessions are uploading, and the per-terminal state uploads need.
+/// Every method takes the public session id.
+#[derive(Default)]
+pub struct Uploads {
+    /// Sessions with an upload running, of any kind: one at a time each.
+    running: Mutex<HashSet<SessionId>>,
+    /// The host each open terminal is connected to, for SFTP uploads.
+    hosts: Mutex<HashMap<SessionId, String>>,
+    /// SFTP connections, opened on a terminal's first SFTP drop and closed with it.
+    sftp: Mutex<HashMap<SessionId, SftpConnection>>,
+}
+
+impl Uploads {
+    /// Marks `id` uploading; refused if it already is.
+    pub fn begin(&self, id: SessionId) -> Result<(), String> {
+        if self
+            .running
+            .lock()
+            .expect("uploads lock poisoned")
+            .insert(id)
+        {
+            Ok(())
+        } else {
+            Err("an upload is already running here".to_string())
+        }
+    }
+
+    pub fn end(&self, id: SessionId) {
+        self.running
+            .lock()
+            .expect("uploads lock poisoned")
+            .remove(&id);
+    }
+
+    /// Records the host terminal `id` is connected to.
+    pub fn terminal_opened(&self, id: SessionId, host_name: &str) {
+        self.hosts
+            .lock()
+            .expect("uploads lock poisoned")
+            .insert(id, host_name.to_string());
+    }
+
+    pub fn terminal_host(&self, id: SessionId) -> Option<String> {
+        self.hosts
+            .lock()
+            .expect("uploads lock poisoned")
+            .get(&id)
+            .cloned()
+    }
+
+    /// Session `id` ended: forget it, and close its SFTP connection.
+    pub fn session_closed(&self, id: SessionId) {
+        self.end(id);
+        self.hosts
+            .lock()
+            .expect("uploads lock poisoned")
+            .remove(&id);
+        let conn = self.sftp.lock().expect("uploads lock poisoned").remove(&id);
+        if let Some(conn) = conn {
+            conn.manager.disconnect();
+        }
+    }
+
+    pub fn has_sftp(&self, id: SessionId) -> bool {
+        self.sftp
+            .lock()
+            .expect("uploads lock poisoned")
+            .contains_key(&id)
+    }
+
+    /// Keeps `manager` for terminal `id`. Hands it back when the terminal
+    /// closed while it was connecting.
+    pub fn add_sftp(&self, id: SessionId, manager: SftpManager) -> Result<(), SftpManager> {
+        if self.terminal_host(id).is_none() {
+            return Err(manager);
+        }
+        self.sftp.lock().expect("uploads lock poisoned").insert(
+            id,
+            SftpConnection {
+                manager,
+                upload: None,
+            },
+        );
+        Ok(())
+    }
+
+    /// Starts `upload` on terminal `id`'s SFTP connection.
+    pub fn start_sftp(&self, id: SessionId, upload: SftpUpload) -> Result<(), String> {
+        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
+        let conn = sftp.get_mut(&id).ok_or("the terminal is closed")?;
+        conn.manager.send(upload.command());
+        conn.upload = Some(upload);
+        Ok(())
+    }
+
+    /// Stops terminal `id`'s SFTP upload after the file being sent.
+    pub fn cancel_sftp(&self, id: SessionId) {
+        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
+        if let Some(upload) = sftp.get_mut(&id).and_then(|c| c.upload.as_mut()) {
+            upload.cancelled = true;
+        }
+    }
+
+    /// A progress report from terminal `id`'s SFTP connection.
+    pub fn sftp_progress(&self, id: SessionId, done: u64, total: u64) -> Option<UploadProgress> {
+        let sftp = self.sftp.lock().expect("uploads lock poisoned");
+        let upload = sftp.get(&id)?.upload.as_ref()?;
+        Some(upload.progress(id, done, total))
+    }
+
+    /// A file on terminal `id`'s SFTP connection is done: sends the next one,
+    /// or returns how the upload ended (and marks `id` no longer uploading).
+    pub fn sftp_done(&self, id: SessionId, result: Result<(), String>) -> Option<UploadResult> {
+        let end = {
+            let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
+            let conn = sftp.get_mut(&id)?;
+            let upload = conn.upload.as_mut()?;
+            match upload.done(result) {
+                Ok(next) => {
+                    conn.manager.send(next);
+                    return None;
+                }
+                Err(end) => {
+                    conn.upload = None;
+                    end
+                }
+            }
+        };
+        self.end(id);
+        Some(end)
+    }
+
+    /// Terminal `id`'s SFTP connection is gone: forget it, and end the upload
+    /// on it, if one was running.
+    pub fn sftp_lost(&self, id: SessionId) -> Option<UploadResult> {
+        let conn = self
+            .sftp
+            .lock()
+            .expect("uploads lock poisoned")
+            .remove(&id)?;
+        conn.upload?;
+        self.end(id);
+        Some(UploadResult {
+            ok: false,
+            message: "the SFTP connection was lost".to_string(),
+        })
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-gui upload 2>&1 | grep -E "^test |test result"; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: `upload::tests` 下 9 个测试通过（加上 Task 1、3 里名字带 upload 的 4 个）。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/src/upload.rs crates/omnyssh-gui/src/events.rs crates/omnyssh-gui/src/main.rs
git commit -m "feat(gui): upload checks, SFTP targets and upload bookkeeping"
```

---

### Task 6: 命令、事件和接线

**Files:** Create `crates/omnyssh-gui/src/commands/upload.rs`；Modify `src/state.rs`、`src/bridge.rs`、`src/commands/serial.rs`、`src/commands/mod.rs`、`src/main.rs`、`ui/src/lib/bindings.ts`

这一步是接线：新的逻辑在 Task 2–5 已经有测试，这里由编译、`committed_bindings_are_in_sync` 和全量测试把关，没有单独的测试补丁。

- [ ] **Step 1: 接线**

用 Write 工具把下面的补丁**原样**存成 `/tmp/p6.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/p6.patch && git apply /tmp/p6.patch`。

```diff
diff --git a/crates/omnyssh-gui/src/bridge.rs b/crates/omnyssh-gui/src/bridge.rs
--- a/crates/omnyssh-gui/src/bridge.rs
+++ b/crates/omnyssh-gui/src/bridge.rs
@@ -4,6 +4,7 @@
 //! forwarder (`forward_terminal_output`) into the per-session channels (§3.6).
 
 use omnyssh_core::event::{CoreEvent, SessionId};
+use omnyssh_core::transfer::{UploadEvent, UploadResult};
 use tauri::{AppHandle, Manager};
 use tauri_specta::Event;
 use tokio::sync::mpsc;
@@ -11,6 +12,7 @@
 use crate::dto::{FileEntryDto, KeyFingerprintDto, TransferProgressDto, TunnelStatusDto};
 use crate::events;
 use crate::state::GuiState;
+use crate::upload;
 
 pub async fn forward_core_events(app: AppHandle, mut rx: mpsc::Receiver<CoreEvent>) {
     while let Some(event) = rx.recv().await {
@@ -110,6 +112,14 @@
                     let _ = events::TerminalExited { session_id }.emit(&app);
                 }
             }
+            // A terminal's upload, by inner PTY id. The core has already shown its
+            // result line in the terminal; a closed tab gets nothing.
+            CoreEvent::Upload(inner_id, event) => {
+                let state = app.state::<GuiState>();
+                if let Some(session_id) = state.terminal_public(inner_id) {
+                    emit_upload(&app, &state, session_id, event);
+                }
+            }
             // Auto key-setup (§4.2): `start_key_setup` drives the core and reports these
             // on the shared engine channel; the host name identifies the run.
             CoreEvent::KeySetupProgress(host_name, step) => {
@@ -154,6 +164,65 @@
         app.state::<GuiState>()
             .send_terminal_output(inner_id, bytes);
     }
+}
+
+/// Emit an upload event of session `session_id`; an ending also frees the
+/// session for the next upload.
+pub fn emit_upload(app: &AppHandle, state: &GuiState, session_id: SessionId, event: UploadEvent) {
+    match event {
+        UploadEvent::Progress(progress) => {
+            if let Some(e) = upload::progress_event(session_id, &progress) {
+                let _ = e.emit(app);
+            }
+        }
+        UploadEvent::Finished(result) => {
+            state.uploads().end(session_id);
+            let _ = finished_event(session_id, result).emit(app);
+        }
+    }
+}
+
+fn finished_event(session_id: SessionId, result: UploadResult) -> events::UploadFinished {
+    events::UploadFinished {
+        session_id,
+        ok: result.ok,
+        message: result.message,
+    }
+}
+
+/// Drains the SFTP connection a terminal uploads through: progress becomes
+/// `upload-progress`, each finished file sends the next, and the end prints
+/// the result line in the terminal. A connection that goes away mid-upload
+/// ends it as lost.
+pub async fn forward_upload_sftp_events(
+    app: AppHandle,
+    session_id: SessionId,
+    mut rx: mpsc::Receiver<CoreEvent>,
+) {
+    let state = app.state::<GuiState>();
+    while let Some(event) = rx.recv().await {
+        let end = match event {
+            CoreEvent::FileTransferProgress(_, done, total) => {
+                if let Some(e) = state.uploads().sftp_progress(session_id, done, total) {
+                    let _ = e.emit(&app);
+                }
+                None
+            }
+            CoreEvent::SftpOpDone { result } => state.uploads().sftp_done(session_id, result),
+            _ => None,
+        };
+        if let Some(result) = end {
+            finish_sftp(&app, &state, session_id, result);
+        }
+    }
+    if let Some(result) = state.uploads().sftp_lost(session_id) {
+        finish_sftp(&app, &state, session_id, result);
+    }
+}
+
+fn finish_sftp(app: &AppHandle, state: &GuiState, session_id: SessionId, result: UploadResult) {
+    state.print_to_terminal(session_id, result.line().as_bytes());
+    let _ = finished_event(session_id, result).emit(app);
 }
 
 /// A typed SFTP event stamped with its owning session id, ready to emit. Built by
diff --git a/crates/omnyssh-gui/src/commands/mod.rs b/crates/omnyssh-gui/src/commands/mod.rs
--- a/crates/omnyssh-gui/src/commands/mod.rs
+++ b/crates/omnyssh-gui/src/commands/mod.rs
@@ -14,3 +14,4 @@
 pub mod tray;
 pub mod tunnels;
 pub mod update;
+pub mod upload;
diff --git a/crates/omnyssh-gui/src/commands/serial.rs b/crates/omnyssh-gui/src/commands/serial.rs
--- a/crates/omnyssh-gui/src/commands/serial.rs
+++ b/crates/omnyssh-gui/src/commands/serial.rs
@@ -1,7 +1,8 @@
 //! Serial port commands. Received bytes stream out on the tab's own raw
 //! `Channel`, like a terminal's; a port that fails reports once on the tab's exit
-//! channel, so nothing goes through the global event bus and no exit can race
-//! ahead of the id the frontend is waiting for.
+//! channel, so no exit can race ahead of the id the frontend is waiting for.
+//! Only uploads report on the global bus, as `upload-*` events like a
+//! terminal's.
 
 use omnyssh_core::config::serial_devices::{
     load_serial_devices, save_serial_devices, SerialDevice,
@@ -10,8 +11,9 @@
 use omnyssh_core::serial::{self, SerialOutput, SerialSession};
 use omnyssh_core::transfer::UploadEvent;
 use tauri::ipc::Channel;
-use tauri::State;
-
+use tauri::{AppHandle, Manager, State};
+
+use crate::bridge;
 use crate::dto::{SerialConfigDto, SerialDeviceDto, SerialExitDto, SerialPortDto, TerminalBytes};
 use crate::error::CommandError;
 use crate::session_log::SessionLogs;
@@ -40,6 +42,7 @@
 #[tauri::command]
 #[specta::specta]
 pub async fn serial_open(
+    app: AppHandle,
     state: State<'_, GuiState>,
     config: SerialConfigDto,
     on_output: Channel<TerminalBytes>,
@@ -60,8 +63,7 @@
                 |error| {
                     let _ = on_exit.send(SerialExitDto { error });
                 },
-                // Nothing starts an upload yet.
-                |_| {},
+                |event| bridge::emit_upload(&app, &app.state::<GuiState>(), id, event),
             )
         })
     })
diff --git a/crates/omnyssh-gui/src/commands/upload.rs b/crates/omnyssh-gui/src/commands/upload.rs
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/src/commands/upload.rs
@@ -0,0 +1,158 @@
+//! Drag-and-drop upload commands. Each checks the drop first (folders, XMODEM's
+//! one file, ZMODEM's limits, an upload already running), so a refused drop
+//! sends nothing. Progress and the end arrive as `upload-progress` and
+//! `upload-finished`; a command that returns `Ok` is always followed by one
+//! `upload-finished`.
+
+use std::time::Instant;
+
+use omnyssh_core::ssh::host_key::HostKeyPrompter;
+use omnyssh_core::ssh::identity;
+use omnyssh_core::ssh::password::Prompter;
+use omnyssh_core::ssh::sftp::SftpManager;
+use omnyssh_core::transfer::{ActiveUpload, Protocol, UploadFile};
+use tauri::{AppHandle, State};
+use tokio::sync::mpsc;
+
+use crate::bridge;
+use crate::dto::{HostUploadProtocolDto, SerialUploadProtocolDto};
+use crate::error::CommandError;
+use crate::state::GuiState;
+use crate::upload::{self, Dropped, Method, SftpFile, SftpUpload, RZ_COMMAND};
+
+/// The upload connection's event buffer, like an SFTP tab's.
+const SFTP_EVENT_BUFFER: usize = 256;
+
+/// Upload `paths` into terminal `session_id`: ZMODEM to `rz` in its shell, or
+/// SFTP into the host's upload folder over a connection kept for the terminal.
+#[tauri::command]
+#[specta::specta]
+pub async fn terminal_upload(
+    app: AppHandle,
+    state: State<'_, GuiState>,
+    session_id: u64,
+    protocol: HostUploadProtocolDto,
+    paths: Vec<String>,
+) -> Result<(), CommandError> {
+    let method = match protocol {
+        HostUploadProtocolDto::Zmodem => Method::Zmodem,
+        HostUploadProtocolDto::Sftp => Method::Sftp,
+    };
+    let dropped = checked(method, &paths)?;
+    state.uploads().begin(session_id).map_err(error)?;
+    let started = match method {
+        Method::Sftp => start_sftp(&app, &state, session_id, &dropped).await,
+        _ => open(&dropped).and_then(|files| {
+            let upload = ActiveUpload::start(Protocol::Zmodem, files, Instant::now());
+            state.upload_terminal(session_id, RZ_COMMAND, upload)
+        }),
+    };
+    if started.is_err() {
+        state.uploads().end(session_id);
+    }
+    started.map_err(error)
+}
+
+/// Upload `paths` to serial port `session_id`. ZMODEM types `rz -b` first;
+/// YMODEM and XMODEM wait for the receiver the user started (`loady`, `rx`).
+#[tauri::command]
+#[specta::specta]
+pub async fn serial_upload(
+    state: State<'_, GuiState>,
+    session_id: u64,
+    protocol: SerialUploadProtocolDto,
+    paths: Vec<String>,
+) -> Result<(), CommandError> {
+    let (method, protocol, preamble): (_, _, &[u8]) = match protocol {
+        SerialUploadProtocolDto::Zmodem => (Method::Zmodem, Protocol::Zmodem, RZ_COMMAND),
+        SerialUploadProtocolDto::Ymodem => (Method::Ymodem, Protocol::Ymodem, b""),
+        SerialUploadProtocolDto::Xmodem => (Method::Xmodem, Protocol::Xmodem, b""),
+    };
+    let dropped = checked(method, &paths)?;
+    state.uploads().begin(session_id).map_err(error)?;
+    let started = open(&dropped).and_then(|files| {
+        let upload = ActiveUpload::start(protocol, files, Instant::now());
+        state.upload_serial(session_id, preamble, upload)
+    });
+    if started.is_err() {
+        state.uploads().end(session_id);
+    }
+    started.map_err(error)
+}
+
+/// Cancel session `session_id`'s upload. ZMODEM, YMODEM and XMODEM stop at once;
+/// SFTP after the file being sent. A session without one is a no-op.
+#[tauri::command]
+#[specta::specta]
+pub fn upload_cancel(state: State<'_, GuiState>, session_id: u64) -> Result<(), CommandError> {
+    state.cancel_upload(session_id);
+    Ok(())
+}
+
+fn checked(method: Method, paths: &[String]) -> Result<Vec<Dropped>, CommandError> {
+    let dropped = upload::inspect(paths).map_err(error)?;
+    upload::check(method, &dropped).map_err(error)?;
+    Ok(dropped)
+}
+
+fn open(dropped: &[Dropped]) -> Result<Vec<UploadFile>, String> {
+    dropped
+        .iter()
+        .map(|d| {
+            UploadFile::open(std::path::Path::new(&d.path))
+                .map_err(|e| format!("cannot read {}: {e}", d.name))
+        })
+        .collect()
+}
+
+/// Sends `dropped` over terminal `id`'s SFTP connection, opening it first.
+async fn start_sftp(
+    app: &AppHandle,
+    state: &GuiState,
+    id: u64,
+    dropped: &[Dropped],
+) -> Result<(), String> {
+    let host_name = state
+        .uploads()
+        .terminal_host(id)
+        .ok_or("the terminal is closed")?;
+    let host = state
+        .host_by_name(&host_name)
+        .ok_or_else(|| format!("unknown host '{host_name}'"))?;
+    if !state.uploads().has_sftp(id) {
+        let (tx, rx) = mpsc::channel(SFTP_EVENT_BUFFER);
+        let prompter = Prompter::new(state.engine_sender(), &host.name);
+        let host_keys = HostKeyPrompter::new(state.engine_sender(), &host.name);
+        let manager =
+            match SftpManager::connect_asking_host_keys(&host, tx, prompter, host_keys).await {
+                Ok(manager) => manager,
+                Err(e) => {
+                    if let Some(path) = omnyssh_core::ssh::session::passphrase_required(&e) {
+                        identity::ask_passphrase(&state.engine_sender(), &host.name, path).await;
+                    }
+                    return Err(format!("{e:#}"));
+                }
+            };
+        if let Err(manager) = state.uploads().add_sftp(id, manager) {
+            manager.disconnect();
+            return Err("the terminal is closed".to_string());
+        }
+        tauri::async_runtime::spawn(bridge::forward_upload_sftp_events(app.clone(), id, rx));
+    }
+    let files = dropped
+        .iter()
+        .map(|d| SftpFile {
+            local: d.path.clone(),
+            remote: upload::sftp_target(host.upload_dir.as_deref(), &d.name),
+            name: d.name.clone(),
+            size: d.size,
+        })
+        .collect();
+    state.uploads().start_sftp(id, SftpUpload::new(files))
+}
+
+fn error(message: impl Into<String>) -> CommandError {
+    CommandError {
+        message: message.into(),
+    }
+}
diff --git a/crates/omnyssh-gui/src/main.rs b/crates/omnyssh-gui/src/main.rs
--- a/crates/omnyssh-gui/src/main.rs
+++ b/crates/omnyssh-gui/src/main.rs
@@ -41,6 +41,7 @@
 use commands::tray::set_tray_behavior;
 use commands::tunnels::{tunnel_start, tunnel_stop};
 use commands::update::{check_update, install_update, load_update_config, save_update_config};
+use commands::upload::{serial_upload, terminal_upload, upload_cancel};
 use omnyssh_core::event::{CoreEvent, SessionId};
 use omnyssh_core::ssh::pty::PtyManager;
 use state::GuiState;
@@ -125,6 +126,9 @@
             list_serial_devices,
             save_serial_device,
             delete_serial_device,
+            terminal_upload,
+            serial_upload,
+            upload_cancel,
             pick_save_path,
             log_start,
             log_stop,
@@ -183,6 +187,8 @@
             events::PasswordRequired,
             events::HostKeyChanged,
             events::LogStopped,
+            events::UploadProgress,
+            events::UploadFinished,
             events::Error
         ])
 }
diff --git a/crates/omnyssh-gui/src/state.rs b/crates/omnyssh-gui/src/state.rs
--- a/crates/omnyssh-gui/src/state.rs
+++ b/crates/omnyssh-gui/src/state.rs
@@ -20,8 +20,11 @@
 use tauri::ipc::Channel;
 use tokio::sync::mpsc;
 
+use omnyssh_core::transfer::ActiveUpload;
+
 use crate::dto::{HostDto, TerminalBytes, TunnelStatusDto};
 use crate::session_log::SessionLogs;
+use crate::upload::Uploads;
 
 /// Metric poll cadence. Mirrors the TUI's fixed interval; a configurable refresh
 /// interval lands with settings in Stage 4.3 (tech-gui.md §4.3).
@@ -119,6 +122,8 @@
     picked_paths: Mutex<HashSet<PathBuf>>,
     /// The folder of the last picked path, where the next save dialog opens.
     save_dir: Mutex<Option<PathBuf>>,
+    /// Drag-and-drop uploads: who is uploading, and each terminal's SFTP connection.
+    uploads: Uploads,
     /// Shared engine channel the bridge drains; cloned to `PollManager`/`PtyManager`.
     engine_tx: mpsc::Sender<CoreEvent>,
 }
@@ -142,6 +147,7 @@
             logs: Arc::new(SessionLogs::default()),
             picked_paths: Mutex::new(HashSet::new()),
             save_dir: Mutex::new(None),
+            uploads: Uploads::default(),
             engine_tx,
         }
     }
@@ -359,11 +365,13 @@
             .lock()
             .expect("term_channels lock poisoned")
             .insert(inner, channel);
-        Ok(self
+        let public = self
             .sessions
             .lock()
             .expect("sessions lock poisoned")
-            .register_terminal(inner))
+            .register_terminal(inner);
+        self.uploads.terminal_opened(public, host_name);
+        Ok(public)
     }
 
     /// Resolve a public id to its live PTY inner id and run `f` on the manager. The
@@ -411,6 +419,7 @@
                 .expect("term_channels lock poisoned")
                 .remove(&inner);
         }
+        self.uploads.session_closed(public);
         self.logs.stop(public);
     }
 
@@ -455,6 +464,7 @@
             .expect("term_channels lock poisoned")
             .remove(&inner);
         if let Some(public) = public {
+            self.uploads.session_closed(public);
             self.logs.stop(public);
         }
         public
@@ -556,10 +566,80 @@
     /// session's callback might take. A port that already failed stays in the map
     /// until this, which is harmless — writes to it go nowhere.
     pub fn take_serial(&self, id: SessionId) -> Option<SerialSession> {
+        self.uploads.session_closed(id);
         self.serial
             .lock()
             .expect("serial lock poisoned")
             .remove(&id)
+    }
+
+    /// Drag-and-drop upload bookkeeping.
+    pub fn uploads(&self) -> &Uploads {
+        &self.uploads
+    }
+
+    /// Run `upload` in terminal `public`, after typing `preamble` there.
+    pub fn upload_terminal(
+        &self,
+        public: SessionId,
+        preamble: &[u8],
+        upload: ActiveUpload,
+    ) -> Result<(), String> {
+        let mut upload = Some(upload);
+        self.on_pty_inner(public, |pty, inner| {
+            if let Some(upload) = upload.take() {
+                pty.upload(inner, preamble, upload);
+            }
+        });
+        match upload {
+            None => Ok(()),
+            Some(_) => Err("the terminal is closed".to_string()),
+        }
+    }
+
+    /// Run `upload` on serial port `id`, after sending `preamble` there.
+    pub fn upload_serial(
+        &self,
+        id: SessionId,
+        preamble: &[u8],
+        upload: ActiveUpload,
+    ) -> Result<(), String> {
+        match self.serial.lock().expect("serial lock poisoned").get(&id) {
+            Some(session) => {
+                session.upload(preamble, upload);
+                Ok(())
+            }
+            None => Err("the port is closed".to_string()),
+        }
+    }
+
+    /// Cancel whatever upload session `id` runs. Unknown ids are a no-op.
+    pub fn cancel_upload(&self, id: SessionId) {
+        self.on_pty_inner(id, |pty, inner| pty.cancel_upload(inner));
+        if let Some(session) = self.serial.lock().expect("serial lock poisoned").get(&id) {
+            session.cancel_upload();
+        }
+        self.uploads.cancel_sftp(id);
+    }
+
+    /// The public id of the terminal with PTY inner id `inner`, while it is open.
+    pub fn terminal_public(&self, inner: SessionId) -> Option<SessionId> {
+        self.sessions
+            .lock()
+            .expect("sessions lock poisoned")
+            .public_of(inner)
+    }
+
+    /// Show `bytes` in terminal `public`, and its log, as if the server sent them.
+    pub fn print_to_terminal(&self, public: SessionId, bytes: &[u8]) {
+        let inner = self
+            .sessions
+            .lock()
+            .expect("sessions lock poisoned")
+            .pty_inner(public);
+        if let Some(inner) = inner {
+            self.send_terminal_output(inner, bytes.to_vec());
+        }
     }
 
     /// The session logs, for a serial session's output callback and the flush tick.
diff --git a/crates/omnyssh-gui/ui/src/lib/bindings.ts b/crates/omnyssh-gui/ui/src/lib/bindings.ts
--- a/crates/omnyssh-gui/ui/src/lib/bindings.ts
+++ b/crates/omnyssh-gui/ui/src/lib/bindings.ts
@@ -264,6 +264,42 @@
 async deleteSerialDevice(name: string) : Promise<Result<null, CommandError>> {
     try {
     return { status: "ok", data: await TAURI_INVOKE("delete_serial_device", { name }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Upload `paths` into terminal `session_id`: ZMODEM to `rz` in its shell, or
+ * SFTP into the host's upload folder over a connection kept for the terminal.
+ */
+async terminalUpload(sessionId: number, protocol: HostUploadProtocolDto, paths: string[]) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("terminal_upload", { sessionId, protocol, paths }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Upload `paths` to serial port `session_id`. ZMODEM types `rz -b` first;
+ * YMODEM and XMODEM wait for the receiver the user started (`loady`, `rx`).
+ */
+async serialUpload(sessionId: number, protocol: SerialUploadProtocolDto, paths: string[]) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("serial_upload", { sessionId, protocol, paths }) };
+} catch (e) {
+    if(e instanceof Error) throw e;
+    else return { status: "error", error: e  as any };
+}
+},
+/**
+ * Cancel session `session_id`'s upload. ZMODEM, YMODEM and XMODEM stop at once;
+ * SFTP after the file being sent. A session without one is a no-op.
+ */
+async uploadCancel(sessionId: number) : Promise<Result<null, CommandError>> {
+    try {
+    return { status: "ok", data: await TAURI_INVOKE("upload_cancel", { sessionId }) };
 } catch (e) {
     if(e instanceof Error) throw e;
     else return { status: "error", error: e  as any };
@@ -690,7 +726,9 @@
 terminalExited: TerminalExited,
 transferProgress: TransferProgress,
 tunnelStatusChanged: TunnelStatusChanged,
-updateAvailable: UpdateAvailable
+updateAvailable: UpdateAvailable,
+uploadFinished: UploadFinished,
+uploadProgress: UploadProgress
 }>({
 error: "error",
 filePreview: "file-preview",
@@ -715,7 +753,9 @@
 terminalExited: "terminal-exited",
 transferProgress: "transfer-progress",
 tunnelStatusChanged: "tunnel-status-changed",
-updateAvailable: "update-available"
+updateAvailable: "update-available",
+uploadFinished: "upload-finished",
+uploadProgress: "upload-progress"
 })
 
 /** user-defined constants **/
@@ -1049,6 +1089,18 @@
  * none is invented (§4.1).
  */
 export type UpdateInfoDto = { version: string; url: string; tag: string; canSelfUpdate: boolean }
+/**
+ * A drag-and-drop upload ended. `message` is what the terminal's result line
+ * says: `uploaded foo.bin (1.2 MB)`, `cancelled`, or why it failed. Ends every
+ * upload a command started, including one whose session went away.
+ */
+export type UploadFinished = { sessionId: number; ok: boolean; message: string }
+/**
+ * How a drag-and-drop upload in a terminal or serial tab is going. `waiting`
+ * while the receiver has not started yet (the other fields are then empty);
+ * `fileIndex` counts from 0. At most every 100 ms per session.
+ */
+export type UploadProgress = { sessionId: number; waiting: boolean; fileIndex: number; fileCount: number; fileName: string; sent: number; total: number }
 
 /** tauri-specta globals **/
 
```

- [ ] **Step 2: 跑测试和检查**

```bash
nohup sh -c 'cargo test --workspace 2>&1 | grep -E "test result|FAILED|panicked|^error"; cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -3; cargo fmt --check && echo FMT_OK; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 所有 `test result` 都是 `ok`（含 `committed_bindings_are_in_sync`）；clippy 以 `Finished` 结束，没有 warning（Task 5 的 dead_code 警告消失）；输出 `FMT_OK`。

- [ ] **Step 3: Commit**

```bash
git add crates/omnyssh-gui/src crates/omnyssh-gui/ui/src/lib/bindings.ts
git commit -m "feat(gui): drag-and-drop upload commands and events"
```

---

### Task 7: 全量检查

- [ ] **Step 1: Rust 和互通测试**

```bash
nohup sh -c 'cargo test --workspace 2>&1 | grep -E "test result|FAILED|panicked"; LRZSZ_DIR=/tmp/lrzsz/usr/bin cargo test -p omnyssh-core --test transfer_lrzsz --test upload_pty -- --include-ignored 2>&1 | grep -E "^test |test result"; cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -1; cargo fmt --check && echo FMT_OK; echo EXIT=$?' > /tmp/p.log 2>&1 &
```
然后反复 `tail -20 /tmp/p.log`，直到出现 `EXIT=`。

Expected: 全部 `ok`；互通测试 5 个全过（`zmodem_to_rz`、`ymodem_to_rb`、`xmodem_to_rx_with_crc_and_with_checksum`、`a_zmodem_upload_through_a_terminal_reaches_rz`、`cancelling_an_upload_the_shell_cannot_take_shows_its_reply`）；clippy 无警告；`FMT_OK`。

- [ ] **Step 2: 前端没有受影响**

在 `crates/omnyssh-gui/ui` 下：

```bash
nohup sh -c 'npm run check 2>&1 | tail -2; npx vitest run 2>&1 | grep -E "Tests "; npx playwright test --reporter=line 2>&1 | tail -3; echo EXIT=$?' > /tmp/p-ui.log 2>&1 &
```
然后反复 `tail -20 /tmp/p-ui.log`，直到出现 `EXIT=`。

Expected: svelte-check `0 ERRORS 0 WARNINGS`；vitest 449 个通过；Playwright 97 个通过。前端代码在这份计划里没有改动，只有生成的 bindings 多了可选字段、命令和事件。

- [ ] **Step 3: 如果有 fmt 改动**

运行 `cargo fmt`，再跑一次 Step 1，然后 `git add -u crates && git commit -m "style: rustfmt"`。没有改动就跳过。
