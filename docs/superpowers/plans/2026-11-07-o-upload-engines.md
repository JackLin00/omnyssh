# O. 拖放上传：传输引擎 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `omnyssh-core` 里新增 `transfer` 模块：三种发送端协议（ZMODEM、YMODEM、XMODEM），都实现同一个 sans-IO 接口 `Upload`，能和真实的 lrzsz（`rz`、`rb`、`rx`）互传。这份计划只做引擎；接入串口和 SSH 会话、GUI 命令和事件、SFTP、前端拖放，都在下一份计划 **P. 拖放上传：接入与界面** 里。

**Architecture:**
- `transfer/mod.rs`：`Protocol`、`UploadFile`（含 `open(path)`）、`UploadState`、`Upload` trait、`start()`，以及测试辅助 `test_util`。
- `transfer/xmodem.rs`：自己实现的 XMODEM/YMODEM 发送端（同一个状态机，`Variant` 区分）。
- `transfer/zmodem.rs`：包装 `zmodem2` 0.7.3 的 `Sender`，补上读文件、计时器、跳过计数、对端 CAN 取消检测和状态上报。
- 引擎不碰端口、通道和系统时钟。调用方把收到的字节交给 `receive`，在自己的发送缓冲排空后调用 `transmit` 取数据，定期调用 `tick(now)`。

**Tech Stack:** Rust 2021（工具链 1.97）、`zmodem2` 0.7.3（新增；依赖 bitflags 2、hex、thiserror 1 都已在依赖树里，`Cargo.lock` 只多出 `zmodem2` 一个包）、`tempfile`（已有的 dev 依赖）。

**Spec:** `docs/superpowers/specs/2026-11-07-drag-drop-upload-design.md`

**命令约定（沙箱限制，务必遵守）：** 前台命令如果 60–100 秒没有输出，会被沙箱杀掉。所有 cargo 命令都要这样跑：
1. 用 `nohup sh -c '…; echo EXIT=$?' > /tmp/o.log 2>&1 &` 放到后台；
2. 反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

cargo 命令都在仓库根目录执行。这台机器磁盘很紧张（`target/` 约 30 GB），**不要**另设 `CARGO_TARGET_DIR`，也不要新建 worktree 去编译，直接用仓库的 `target/`。

---

## 实现中确定的设计决定

> 写这份计划时，整套改动已经在仓库的临时副本里完整实现并验证过：`cargo test -p omnyssh-core`（307 个库测试和全部集成测试）、`cargo test -p omnyssh-core --test transfer_lrzsz -- --ignored`（对 lrzsz 0.12.21）、`cargo clippy -p omnyssh-core --all-targets -- -D warnings`、`cargo fmt --check -p omnyssh-core` 都通过。下面的代码就是副本里的代码。

1. **ZMODEM 用 `zmodem2`，`rz` 用 `-b` 不加 `-e`（已实测）**：
   - `zmodem2` 0.7.3 是 sans-IO 的：`poll()` 返回 `WriteWire` / `ReadFile` / `Event` / `Idle`，和我们的接口一一对应。
   - spike 中经管道对 `rz -b` 发 1 MB 随机文件、小文件、空文件，SHA-256 一致，用时约 0.17 s。
   - `rz -be`（要求转义全部控制字符）握手失败：`zmodem2` 不支持 ESCCTL。标准 ZDLE 转义已经覆盖 ZDLE、DLE、XON、XOFF（以及它们的高位形式），软件流控的串口也安全。所以 P 计划里 GUI 写入的命令是 `rz -b\r`。
2. **`FileInfo` 带上 mtime 和 mode**：不带 mtime 时，`rz` 把文件时间写成乱值（spike 中出现 2446 年）；不带 mode 时文件是 `0600`。所以发 `with_modified(mtime)` 和 `with_mode(0o100644)`。mtime 超出 `u32` 时不发。
3. **跳过的判断**：`zmodem2` 收到 `ZSKIP` 只报 `FileCompleted`。规则：一个大小大于 0 的文件在 `FileCompleted` 之前没有任何 `ReadFile`，就记为跳过。互通测试里第二次上传同样两个文件得到 `Done { sent_files: 0, skipped: 2 }`。
4. **对端取消要自己检测**：`rz` 被 Ctrl+C 时发 10 个 CAN（0x18，也就是 ZDLE）加 10 个退格，不是 ZABORT/ZCAN 帧，`zmodem2` 认不出来。我们在 `receive` 里数连续的 CAN，5 个就算取消（和 lrzsz 的阈值一样，正常数据流里不会出现）。第 5 个 CAN 之后的字节进 `leftover`，交还给终端。
5. **计时器**（`zmodem2` 没有计时器）：
   - 握手：10 s 内没有 ZRINIT 就失败，提示检查 lrzsz；等待期间每 2 s 重发一次 ZRQINIT（`Sender::timeout()`）。
   - "已经握手"的判定：出现过 `ReadFile` 或 `FileCompleted`。`zmodem2` 不单独报告收到 ZRINIT。
   - 握手之后接收端 30 s 没有任何字节就失败。`rz` 每几个子包确认一次，只有链路断了才会这么久没动静。
6. **XMODEM/YMODEM 细节**：
   - 接收端发 `C` 就用 CRC-16 和 1K 块（剩余不超过 128 字节时，最后一块用 128 字节块，少填充）；发 NAK 就用 8 位校验和和 128 字节块。模式由第一个文件开始时决定。
   - 最后一块用 0x1A 填充。YMODEM 的 0 号块是 `名字 NUL 大小 空格 mtime(八进制) NUL`，用 0 填充；信息超过 128 字节时用 1K 的 0 号块。批量结束用全 0 的 0 号块。
   - YMODEM 接收端（`rb`）对第一个 EOT 回 NAK，第二个才回 ACK，所以 EOT 被 NAK 时照常重发。
   - 等应答时，`C` 和其他杂字节一概忽略。如果把迟到的 `C` 当作 NAK 去重发 1 号块，接收端会多回一个 ACK，被我们当成 2 号块的确认，传输就错位了。
   - 每块等 10 s，超时重发；同一块重发超过 10 次就失败（`too many retries at block N`），并发 `CAN CAN`。连续收到两个 CAN 算对端取消。
   - 等接收端开始：第一个文件等 60 s（用户还要在设备上输入 `loady`），提示里点名 `loady/loadx`；后续文件或数据开始等 10 s。
7. **取消和失败时的输出**：
   - XMODEM/YMODEM：清掉还没发的字节，改为发 `CAN CAN`。
   - ZMODEM：`fail()` 清掉我们自己缓冲里还没交出去的字节，再调用 `Sender::abort()`，后者发 10 个 CAN 加 10 个退格。
   - 进入最终状态后，`receive` 收到的字节全部进 `leftover`。正常结束（`SessionCompleted`）时，没处理完的输入也进 `leftover`（里面可能有 `rz` 最后一帧尾部的 CR LF，无害）。中止（`Aborted`）时，没处理完的输入直接丢掉，因为那是协议字节，显示在终端里只是乱码。
8. **消息用英文**：界面文字都是英文（Log、Export、Show in folder），所以 `UploadState::Failed` 里的消息也用英文。spec 里的中文提示只是意思，实际文本以这里为准：
   - `the device did not answer rz (is lrzsz installed?)`
   - `the receiver did not start (did you run loady/loadx on the device?)`
   - `the device stopped responding`
   - `the device cancelled the transfer`
   - `too many retries at block N` / `at end of file` / `at the file header`
   - `could not read NAME: ERROR` / `the file got shorter while it was being sent`
   - `NAME is larger than ZMODEM's 4 GiB limit`
9. **`transmit` 每次最多 8 KiB**（`TRANSMIT_CHUNK`）。调用方只在自己的缓冲排空后才再要，这样串口线程的 `pending` 有上限，SSH 的 `channel.data` 也不会一次压太多。
10. **接口与 spec 的出入**：`UploadFile.reader` 是 `Box<dyn Source>`（`Read + Seek + Send`），不是 `Read + Send`，因为 ZMODEM 会按接收端的 ZRPOS 回到某个偏移重发。另外加了 `modified` 字段，以及 `UploadFile::open`、`Protocol::batch`、`UploadState::is_final` 三个小工具。

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `crates/omnyssh-core/Cargo.toml` | 新增 `zmodem2 = "0.7.3"` |
| `crates/omnyssh-core/src/lib.rs` | `pub mod transfer;` |
| `crates/omnyssh-core/src/transfer/mod.rs`（新建） | 公共类型、`Upload` trait、`start()`、`test_util` |
| `crates/omnyssh-core/src/transfer/xmodem.rs`（新建） | XMODEM/YMODEM 发送端及其测试（含模拟接收端） |
| `crates/omnyssh-core/src/transfer/zmodem.rs`（新建） | ZMODEM 发送端及其测试（用 `zmodem2::Receiver` 当对端） |
| `crates/omnyssh-core/tests/transfer_lrzsz.rs`（新建） | 和真实 lrzsz 的互通测试，`#[ignore]` |

---

### Task 1: 依赖、模块骨架和公共类型

**Files:**
- Modify: `crates/omnyssh-core/Cargo.toml`
- Modify: `crates/omnyssh-core/src/lib.rs`
- Create: `crates/omnyssh-core/src/transfer/mod.rs`

- [ ] **Step 1: 加依赖**

在 `crates/omnyssh-core/Cargo.toml` 的 `serialport = …` 那一段之后、`# Terminal emulator` 之前插入：

```toml
# ZMODEM for drag-and-drop uploads to `rz`: a sans-IO sender, so it runs
# inside the serial thread and the SSH pump alike. Its dependencies (bitflags,
# hex, thiserror 1) are already built.
zmodem2 = "0.7.3"

```

- [ ] **Step 2: 注册模块**

`crates/omnyssh-core/src/lib.rs` 里，在 `pub mod ssh;` 之后加一行：

```rust
pub mod transfer;
```

- [ ] **Step 3: 写 `transfer/mod.rs`（含测试）**

这一步还没有 `mod xmodem; mod zmodem;` 和 `start()`，Task 2、Task 3 再加。Task 2 之前，`TRANSMIT_CHUNK` 和 `test_util` 里的 `file`、`bytes`、`drain` 都还没人用，编译会有 4 个 dead_code 警告，属于正常（已实测）。

```rust
//! Sending files to a device over a terminal byte stream: ZMODEM to `rz`,
//! YMODEM and XMODEM to bootloaders such as U-Boot's `loady`/`loadx`.
//!
//! Each protocol is a sans-IO state machine behind [`Upload`]: the session
//! that owns the port or channel feeds it what the device sends, takes what it
//! has to send, and drives its timers with the current time. Nothing here
//! touches a port, a channel or the clock, so the tests run the protocols
//! against in-memory receivers with a hand-advanced `Instant`.

use std::fs::File;
use std::io::{self, Read, Seek};
use std::path::Path;
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How much one [`Upload::transmit`] call adds at most. The caller only asks
/// again once it has sent it all, so this bounds what sits in its buffer.
pub(crate) const TRANSMIT_CHUNK: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Zmodem,
    Ymodem,
    Xmodem,
}

impl Protocol {
    /// Whether one upload can carry several files.
    pub fn batch(self) -> bool {
        !matches!(self, Protocol::Xmodem)
    }
}

/// A file's contents, read and re-read (ZMODEM resends from an offset the
/// receiver asks for).
pub trait Source: Read + Seek + Send {}
impl<T: Read + Seek + Send> Source for T {}

pub struct UploadFile {
    /// The name the receiver saves it under: the base name, no directories.
    pub name: String,
    pub size: u64,
    /// Last change, in seconds since the Unix epoch.
    pub modified: Option<u64>,
    pub reader: Box<dyn Source>,
}

impl UploadFile {
    /// Opens a local regular file for upload.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Ok(Self {
            name,
            size: meta.len(),
            modified,
            reader: Box::new(file),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadState {
    /// Waiting for the receiver to start (ZRINIT, `C` or NAK).
    Waiting,
    Sending {
        /// 0-based.
        file_index: usize,
        file_count: usize,
        file_name: String,
        sent: u64,
        total: u64,
    },
    Done {
        sent_files: usize,
        skipped: usize,
    },
    Failed(String),
    Cancelled,
}

impl UploadState {
    /// Done, failed or cancelled: the session leaves upload mode once
    /// [`Upload::transmit`] has nothing more to send.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            UploadState::Done { .. } | UploadState::Failed(_) | UploadState::Cancelled
        )
    }
}

/// One upload in progress. Contract for the caller:
/// - pass every byte the device sends to `receive`;
/// - call `tick` regularly (every 100 ms or so) and after `receive`;
/// - call `transmit` whenever its own send buffer is empty, and send what it
///   adds;
/// - once `state()` is final and `transmit` adds nothing, hand `leftover()` to
///   the terminal and stop routing bytes here.
pub trait Upload: Send {
    fn receive(&mut self, data: &[u8]);
    fn transmit(&mut self, out: &mut Vec<u8>);
    fn tick(&mut self, now: Instant);
    /// Stops at once; the cancel sequence comes out of the next `transmit`.
    fn cancel(&mut self);
    fn state(&self) -> UploadState;
    /// What the device sent after the transfer ended (rz's trailing output,
    /// the shell prompt), to show in the terminal.
    fn leftover(&mut self) -> Vec<u8>;
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use std::io::Cursor;

    pub fn file(name: &str, data: Vec<u8>) -> UploadFile {
        UploadFile {
            name: name.to_string(),
            size: data.len() as u64,
            modified: Some(1_700_000_000),
            reader: Box::new(Cursor::new(data)),
        }
    }

    /// Deterministic pseudo-random bytes.
    pub fn bytes(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    /// Takes everything the upload has to send right now.
    pub fn drain(up: &mut dyn Upload) -> Vec<u8> {
        let mut all = Vec::new();
        loop {
            let mut out = Vec::new();
            up.transmit(&mut out);
            if out.is_empty() {
                return all;
            }
            all.extend(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_reads_name_size_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fw.bin");
        std::fs::write(&path, b"12345").unwrap();
        let f = UploadFile::open(&path).unwrap();
        assert_eq!(f.name, "fw.bin");
        assert_eq!(f.size, 5);
        assert!(f.modified.is_some());
    }

    #[test]
    fn open_rejects_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(UploadFile::open(dir.path()).is_err());
    }

    #[test]
    fn only_xmodem_is_single_file() {
        assert!(Protocol::Zmodem.batch());
        assert!(Protocol::Ymodem.batch());
        assert!(!Protocol::Xmodem.batch());
    }
}
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: `transfer::tests` 下 3 个测试通过（`open_reads_name_size_and_time`、`open_rejects_a_directory`、`only_xmodem_is_single_file`）。`Cargo.lock` 新增 `zmodem2 0.7.3` 一个包。

- [ ] **Step 5: Commit**

```bash
git add Cargo.lock crates/omnyssh-core/Cargo.toml crates/omnyssh-core/src/lib.rs crates/omnyssh-core/src/transfer/mod.rs
git commit -m "feat(core): upload types and the sans-IO Upload trait"
```

---

### Task 2: XMODEM / YMODEM 发送端

**Files:**
- Create: `crates/omnyssh-core/src/transfer/xmodem.rs`
- Modify: `crates/omnyssh-core/src/transfer/mod.rs`

- [ ] **Step 1: 先写测试**

在 `transfer/mod.rs` 的 `use serde::…;` 之后加：

```rust
mod xmodem;
```

新建 `crates/omnyssh-core/src/transfer/xmodem.rs`，**先只放测试模块**（下面是文件的末尾部分）。测试里的 `Receiver` 是一个模拟接收端，按 lrzsz 的 `rb`/`rx` 的方式应答：校验每个包的块号补码和 CRC/校验和；YMODEM 第一个 EOT 回 NAK；只允许重复上一块，乱序就 panic。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::test_util::{bytes, drain, file};

    /// A receiver for the tests: checks every packet and answers as lrzsz's
    /// `rb`/`rx` do.
    struct Receiver {
        ymodem: bool,
        crc: bool,
        buf: Vec<u8>,
        /// (name, size from block 0, data)
        files: Vec<(String, Option<u64>, Vec<u8>)>,
        /// Block 0s seen, raw.
        headers: Vec<Vec<u8>>,
        /// First byte of every data packet.
        kinds: Vec<u8>,
        expect: u8,
        eots: u32,
        in_file: bool,
        done: bool,
    }

    impl Receiver {
        fn new(ymodem: bool, crc: bool) -> Self {
            Self {
                ymodem,
                crc,
                buf: Vec::new(),
                files: Vec::new(),
                headers: Vec::new(),
                kinds: Vec::new(),
                expect: 1,
                eots: 0,
                in_file: !ymodem,
                done: false,
            }
        }

        fn start_byte(&self) -> u8 {
            if self.crc {
                CRC_START
            } else {
                NAK
            }
        }

        fn feed(&mut self, data: &[u8]) -> Vec<u8> {
            self.buf.extend_from_slice(data);
            let mut reply = Vec::new();
            loop {
                let Some(&first) = self.buf.first() else {
                    return reply;
                };
                if first == EOT {
                    self.buf.remove(0);
                    if self.ymodem && self.eots == 0 {
                        self.eots = 1;
                        reply.push(NAK);
                    } else {
                        self.eots = 0;
                        reply.push(ACK);
                        self.in_file = false;
                        if self.ymodem {
                            reply.push(CRC_START);
                        } else {
                            self.done = true;
                        }
                    }
                    continue;
                }
                let len = match first {
                    SOH => 128,
                    STX => 1024,
                    other => panic!("unexpected byte {other:#x}"),
                };
                let total = 3 + len + if self.crc { 2 } else { 1 };
                if self.buf.len() < total {
                    return reply;
                }
                let p: Vec<u8> = self.buf.drain(..total).collect();
                assert_eq!(p[1], !p[2], "block number complement");
                let data = &p[3..3 + len];
                if self.crc {
                    assert_eq!(crc16(data).to_be_bytes(), [p[3 + len], p[4 + len]]);
                } else {
                    let sum = data.iter().fold(0u8, |s, &b| s.wrapping_add(b));
                    assert_eq!(sum, p[3 + len]);
                }
                if self.ymodem && !self.in_file {
                    assert_eq!(p[1], 0, "block 0 between files");
                    self.headers.push(data.to_vec());
                    reply.push(ACK);
                    if data[0] == 0 {
                        self.done = true;
                        continue;
                    }
                    let nul = data.iter().position(|&b| b == 0).unwrap();
                    let name = String::from_utf8(data[..nul].to_vec()).unwrap();
                    let rest = &data[nul + 1..];
                    let end = rest.iter().position(|&b| b == b' ' || b == 0).unwrap();
                    let size = std::str::from_utf8(&rest[..end]).unwrap().parse().unwrap();
                    self.files.push((name, Some(size), Vec::new()));
                    self.in_file = true;
                    self.expect = 1;
                    reply.push(CRC_START);
                    continue;
                }
                if self.files.is_empty() {
                    self.files.push(("".into(), None, Vec::new()));
                }
                if p[1] == self.expect {
                    self.kinds.push(first);
                    self.files.last_mut().unwrap().2.extend_from_slice(data);
                    self.expect = self.expect.wrapping_add(1);
                } else {
                    assert_eq!(
                        p[1],
                        self.expect.wrapping_sub(1),
                        "only a repeat may be out of order"
                    );
                }
                reply.push(ACK);
            }
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// Runs an upload against `rx` until neither side has anything to say.
    fn run(up: &mut dyn Upload, rx: &mut Receiver) {
        up.receive(&[rx.start_byte()]);
        for _ in 0..100_000 {
            let out = drain(up);
            if out.is_empty() {
                return;
            }
            let reply = rx.feed(&out);
            up.receive(&reply);
        }
        panic!("the upload did not finish");
    }

    #[test]
    fn crc16_check_value() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn xmodem_crc_sends_1k_blocks_and_pads_the_last() {
        let data = bytes(3000, 1);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t0());
        let mut rx = Receiver::new(false, true);
        run(&mut up, &mut rx);
        assert!(rx.done);
        assert_eq!(rx.kinds, vec![STX, STX, STX]);
        let got = &rx.files[0].2;
        assert_eq!(got.len(), 3072);
        assert_eq!(&got[..3000], &data[..]);
        assert!(got[3000..].iter().all(|&b| b == PAD));
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_short_tail_goes_in_a_128_byte_block() {
        let data = bytes(1100, 2);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t0());
        let mut rx = Receiver::new(false, true);
        run(&mut up, &mut rx);
        assert_eq!(rx.kinds, vec![STX, SOH]);
        assert_eq!(&rx.files[0].2[..1100], &data[..]);
    }

    #[test]
    fn nak_start_falls_back_to_checksum_and_128_byte_blocks() {
        let data = bytes(300, 3);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t0());
        let mut rx = Receiver::new(false, false);
        run(&mut up, &mut rx);
        assert!(rx.done);
        assert_eq!(rx.kinds, vec![SOH, SOH, SOH]);
        assert_eq!(&rx.files[0].2[..300], &data[..]);
    }

    #[test]
    fn ymodem_sends_a_batch_with_names_sizes_and_an_end_block() {
        let a = bytes(5000, 4);
        let b = bytes(10, 5);
        let files = vec![
            file("a.bin", a.clone()),
            file("b.txt", b.clone()),
            file("empty", vec![]),
        ];
        let mut up = XmodemUpload::new(Variant::Ymodem, files, t0());
        let mut rx = Receiver::new(true, true);
        run(&mut up, &mut rx);
        assert!(rx.done);
        let names: Vec<_> = rx.files.iter().map(|f| (f.0.as_str(), f.1)).collect();
        assert_eq!(
            names,
            vec![
                ("a.bin", Some(5000)),
                ("b.txt", Some(10)),
                ("empty", Some(0))
            ]
        );
        assert_eq!(&rx.files[0].2[..5000], &a[..]);
        assert_eq!(&rx.files[1].2[..10], &b[..]);
        assert!(rx.files[2].2.is_empty());
        // name NUL size SP mtime(octal) NUL, zero-filled
        let mut want = b"a.bin\x005000 ".to_vec();
        want.extend_from_slice(format!("{:o}", 1_700_000_000u64).as_bytes());
        want.push(0);
        assert_eq!(&rx.headers[0][..want.len()], &want[..]);
        assert!(rx.headers[0][want.len()..].iter().all(|&b| b == 0));
        assert!(rx.headers[3].iter().all(|&b| b == 0), "end block is empty");
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 3,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_long_name_gets_a_1k_block_0() {
        let name = "n".repeat(200);
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file(&name, vec![1])], t0());
        let mut out = Vec::new();
        up.receive(b"C");
        up.transmit(&mut out);
        assert_eq!(out[0], STX);
        assert_eq!(out.len(), 3 + 1024 + 2);
    }

    #[test]
    fn nak_resends_the_same_block_and_ack_moves_on() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 6))], t0());
        up.receive(b"C");
        let first = drain(&mut up);
        up.receive(&[NAK]);
        assert_eq!(drain(&mut up), first);
        up.receive(&[ACK]);
        let second = drain(&mut up);
        assert_eq!(second[1], 2);
        match up.state() {
            UploadState::Sending { sent, total, .. } => assert_eq!((sent, total), (1024, 2048)),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn unanswered_blocks_are_resent_then_the_upload_gives_up() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(10, 7))], t);
        up.tick(t);
        up.receive(b"C");
        let block = drain(&mut up);
        let mut now = t;
        for _ in 0..MAX_RETRIES {
            now += ACK_TIMEOUT;
            up.tick(now);
            assert_eq!(drain(&mut up), block);
        }
        now += ACK_TIMEOUT;
        up.tick(now);
        assert_eq!(drain(&mut up), vec![CAN, CAN]);
        assert_eq!(
            up.state(),
            UploadState::Failed("too many retries at block 1".into())
        );
    }

    #[test]
    fn a_receiver_that_never_starts_times_out() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file("a", vec![1])], t);
        up.tick(t + START_TIMEOUT - Duration::from_millis(1));
        assert_eq!(up.state(), UploadState::Waiting);
        up.tick(t + START_TIMEOUT);
        match up.state() {
            UploadState::Failed(m) => assert!(m.contains("loady"), "{m}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn two_cans_from_the_device_cancel_and_one_does_not() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(4096, 8))], t0());
        up.receive(b"C");
        drain(&mut up);
        up.receive(&[CAN, ACK]);
        assert!(!up.state().is_final());
        drain(&mut up);
        up.receive(&[CAN, CAN]);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device cancelled the transfer".into())
        );
        assert!(drain(&mut up).is_empty());
    }

    #[test]
    fn cancel_sends_can_can_and_later_bytes_are_leftover() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(4096, 9))], t0());
        up.receive(b"C");
        drain(&mut up);
        up.cancel();
        assert_eq!(up.state(), UploadState::Cancelled);
        assert_eq!(drain(&mut up), vec![CAN, CAN]);
        up.receive(b"=> ");
        assert_eq!(up.leftover(), b"=> ");
    }

    #[test]
    fn bytes_after_the_last_ack_are_leftover() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", vec![1, 2, 3])], t0());
        up.receive(b"C");
        drain(&mut up);
        up.receive(&[ACK]);
        assert_eq!(drain(&mut up), vec![EOT]);
        up.receive(b"\x06## Total Size = 0x3\r\n=> ");
        assert!(up.state().is_final());
        assert_eq!(up.leftover(), b"## Total Size = 0x3\r\n=> ");
    }

    #[test]
    fn noise_before_the_start_is_ignored() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", vec![1])], t0());
        up.receive(b"## Ready for binary (ymodem) download\r\n");
        assert!(drain(&mut up).is_empty());
        up.receive(b"C");
        assert_eq!(drain(&mut up)[0], SOH);
    }

    #[test]
    fn a_read_error_cancels_the_receiver() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk gone"))
            }
        }
        impl std::io::Seek for Broken {
            fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
                Ok(0)
            }
        }
        let f = UploadFile {
            name: "a".into(),
            size: 10,
            modified: None,
            reader: Box::new(Broken),
        };
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![f], t0());
        up.receive(b"C");
        assert_eq!(drain(&mut up), vec![CAN, CAN]);
        assert_eq!(
            up.state(),
            UploadState::Failed("could not read a: disk gone".into())
        );
    }
}
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer::xmodem; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: 编译失败，报 `XmodemUpload`、`Variant`、`crc16`、`SOH` 等找不到。

- [ ] **Step 3: 写实现**

把下面的代码放在 `xmodem.rs` 的**最前面**（测试模块之前），也就是说文件内容 = 这段实现 + 一个空行 + Step 1 的测试模块：

```rust
//! XMODEM and YMODEM senders. They share blocks, CRCs and the ACK/NAK loop;
//! YMODEM adds a block 0 with the file's name and size before each file, and
//! an empty block 0 after the last one.
//!
//! The receiver picks the check: `C` asks for CRC-16, and then data goes in
//! 1024-byte blocks (128 for a short tail); NAK asks for the 8-bit checksum
//! and 128-byte blocks, as the original XMODEM.

use std::io::Read;
use std::time::{Duration, Instant};

use super::{Upload, UploadFile, UploadState, TRANSMIT_CHUNK};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_START: u8 = b'C';
/// Fills the last data block (CP/M's end of file).
const PAD: u8 = 0x1A;

/// How long the receiver gets to start: the user still has to type `loady`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one block waits for its answer before it is sent again.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Sends of one block after the first, before the transfer gives up.
const MAX_RETRIES: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    Xmodem,
    Ymodem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for `C`/NAK to begin the next file (YMODEM: its block 0, or
    /// the end block after the last file; XMODEM: block 1).
    Start,
    /// YMODEM block 0 is out.
    HeaderAck,
    /// YMODEM block 0 was ACKed; waiting for the `C` that asks for data.
    DataStart,
    /// A data block is out.
    BlockAck,
    /// EOT is out.
    EotAck,
    /// The empty YMODEM block 0 that ends the batch is out.
    EndAck,
    Final,
}

pub(crate) struct XmodemUpload {
    variant: Variant,
    files: Vec<UploadFile>,
    /// The file being sent; `files.len()` once all are.
    index: usize,
    /// CRC-16 and 1K blocks, or the checksum and 128-byte blocks.
    crc: bool,
    phase: Phase,
    /// The packet waiting for an answer, to send again.
    packet: Vec<u8>,
    /// Number of the next data block (wraps at 256; starts at 1).
    seq: u8,
    /// Data blocks of this file sent so far, for messages.
    blocks: u32,
    /// File bytes in `packet`.
    packet_data: u64,
    /// File bytes the receiver has ACKed.
    acked: u64,
    retries: u32,
    deadline: Instant,
    now: Instant,
    out: Vec<u8>,
    /// The last byte received was CAN; two in a row cancel.
    saw_can: bool,
    state: UploadState,
    leftover: Vec<u8>,
}

impl XmodemUpload {
    pub(crate) fn new(variant: Variant, files: Vec<UploadFile>, now: Instant) -> Self {
        Self {
            variant,
            files,
            index: 0,
            crc: true,
            phase: Phase::Start,
            packet: Vec::new(),
            seq: 1,
            blocks: 0,
            packet_data: 0,
            acked: 0,
            retries: 0,
            deadline: now + START_TIMEOUT,
            now,
            out: Vec::new(),
            saw_can: false,
            state: UploadState::Waiting,
            leftover: Vec::new(),
        }
    }

    fn on_byte(&mut self, b: u8) {
        if b == CAN {
            if self.saw_can {
                self.finish(UploadState::Failed(
                    "the device cancelled the transfer".into(),
                ));
                return;
            }
            self.saw_can = true;
            return;
        }
        self.saw_can = false;
        let start = b == CRC_START || b == NAK;
        match self.phase {
            Phase::Start if start => {
                if self.index == 0 {
                    self.crc = b == CRC_START;
                }
                match self.variant {
                    Variant::Xmodem => self.send_data_block(),
                    Variant::Ymodem if self.index < self.files.len() => self.send_header(),
                    Variant::Ymodem => {
                        let packet = self.block(0, &[], 128, 0);
                        self.send_packet(packet, Phase::EndAck);
                    }
                }
            }
            Phase::HeaderAck if b == ACK => {
                self.phase = Phase::DataStart;
                self.deadline = self.now + ACK_TIMEOUT;
            }
            Phase::DataStart if start => self.send_data_block(),
            Phase::BlockAck if b == ACK => {
                self.acked += self.packet_data;
                self.retries = 0;
                self.send_data_block();
            }
            Phase::EotAck if b == ACK => {
                self.index += 1;
                self.retries = 0;
                if self.variant == Variant::Xmodem {
                    self.finish(UploadState::Done {
                        sent_files: 1,
                        skipped: 0,
                    });
                } else {
                    self.phase = Phase::Start;
                    self.deadline = self.now + ACK_TIMEOUT;
                }
            }
            Phase::EndAck if b == ACK => self.finish(UploadState::Done {
                sent_files: self.files.len(),
                skipped: 0,
            }),
            Phase::HeaderAck | Phase::BlockAck | Phase::EotAck | Phase::EndAck if b == NAK => {
                self.resend()
            }
            // Anything else (a banner, line noise, an echo) is not an answer.
            _ => {}
        }
    }

    /// YMODEM block 0: `name NUL size [mtime]` (size decimal, mtime octal).
    fn send_header(&mut self) {
        let file = &self.files[self.index];
        let mut info = file.name.as_bytes().to_vec();
        info.push(0);
        info.extend_from_slice(file.size.to_string().as_bytes());
        if let Some(t) = file.modified {
            info.extend_from_slice(format!(" {t:o}").as_bytes());
        }
        info.push(0);
        let len = if info.len() > 128 { 1024 } else { 128 };
        let packet = self.block(0, &info, len, 0);
        self.start_file();
        self.send_packet(packet, Phase::HeaderAck);
    }

    /// The next data block of the current file, or EOT after the last.
    fn send_data_block(&mut self) {
        self.start_file();
        let size = self.files[self.index].size;
        let remaining = size - self.acked;
        if remaining == 0 {
            self.send_packet(vec![EOT], Phase::EotAck);
            return;
        }
        let len = if self.crc && remaining > 128 {
            1024
        } else {
            128
        };
        let want = remaining.min(len as u64) as usize;
        let mut data = vec![0u8; want];
        let name = self.files[self.index].name.clone();
        if let Err(e) = read_full(&mut self.files[self.index].reader, &mut data) {
            self.fail(format!("could not read {name}: {e}"));
            return;
        }
        let packet = self.block(self.seq, &data, len, PAD);
        self.seq = self.seq.wrapping_add(1);
        self.blocks += 1;
        self.packet_data = want as u64;
        self.send_packet(packet, Phase::BlockAck);
    }

    /// Resets the per-file counters when a file begins (called from `Start`).
    fn start_file(&mut self) {
        if self.phase == Phase::Start {
            self.seq = 1;
            self.blocks = 0;
            self.acked = 0;
            self.packet_data = 0;
            let file = &self.files[self.index];
            self.state = UploadState::Sending {
                file_index: self.index,
                file_count: self.files.len(),
                file_name: file.name.clone(),
                sent: 0,
                total: file.size,
            };
        }
    }

    fn block(&self, seq: u8, data: &[u8], len: usize, pad: u8) -> Vec<u8> {
        let mut p = Vec::with_capacity(len + 5);
        p.push(if len == 1024 { STX } else { SOH });
        p.push(seq);
        p.push(!seq);
        let start = p.len();
        p.extend_from_slice(data);
        p.resize(start + len, pad);
        if self.crc {
            let c = crc16(&p[start..]);
            p.extend_from_slice(&c.to_be_bytes());
        } else {
            let sum = p[start..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
            p.push(sum);
        }
        p
    }

    fn send_packet(&mut self, packet: Vec<u8>, phase: Phase) {
        self.out.extend_from_slice(&packet);
        self.packet = packet;
        self.phase = phase;
        self.deadline = self.now + ACK_TIMEOUT;
    }

    fn resend(&mut self) {
        self.retries += 1;
        if self.retries > MAX_RETRIES {
            let what = match self.phase {
                Phase::BlockAck => format!("block {}", self.blocks),
                Phase::EotAck => "end of file".to_string(),
                _ => "the file header".to_string(),
            };
            self.fail(format!("too many retries at {what}"));
            return;
        }
        let packet = self.packet.clone();
        self.out.extend_from_slice(&packet);
        self.deadline = self.now + ACK_TIMEOUT;
    }

    /// Our side gives up: tell the receiver too.
    fn fail(&mut self, message: String) {
        self.out.clear();
        self.out.extend_from_slice(&[CAN, CAN]);
        self.finish(UploadState::Failed(message));
    }

    fn finish(&mut self, state: UploadState) {
        self.phase = Phase::Final;
        self.state = state;
    }
}

impl Upload for XmodemUpload {
    fn receive(&mut self, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            if self.phase == Phase::Final {
                self.leftover.extend_from_slice(&data[i..]);
                return;
            }
            self.on_byte(b);
        }
    }

    fn transmit(&mut self, out: &mut Vec<u8>) {
        let n = self.out.len().min(TRANSMIT_CHUNK);
        out.extend(self.out.drain(..n));
    }

    fn tick(&mut self, now: Instant) {
        self.now = now;
        if now < self.deadline {
            return;
        }
        match self.phase {
            Phase::Start if self.index == 0 => self
                .fail("the receiver did not start (did you run loady/loadx on the device?)".into()),
            Phase::Start | Phase::DataStart => self.fail("the device stopped responding".into()),
            Phase::HeaderAck | Phase::BlockAck | Phase::EotAck | Phase::EndAck => self.resend(),
            Phase::Final => {}
        }
    }

    fn cancel(&mut self) {
        if self.phase == Phase::Final {
            return;
        }
        self.out.clear();
        self.out.extend_from_slice(&[CAN, CAN]);
        self.finish(UploadState::Cancelled);
    }

    fn state(&self) -> UploadState {
        match &self.state {
            UploadState::Sending {
                file_index,
                file_count,
                file_name,
                total,
                ..
            } => UploadState::Sending {
                file_index: *file_index,
                file_count: *file_count,
                file_name: file_name.clone(),
                sent: self.acked,
                total: *total,
            },
            s => s.clone(),
        }
    }

    fn leftover(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.leftover)
    }
}

/// Reads exactly `buf.len()` bytes; a file that ends early is an error.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<()> {
    r.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            std::io::Error::new(e.kind(), "the file got shorter while it was being sent")
        } else {
            e
        }
    })
}

/// CRC-16/XMODEM: polynomial 0x1021, initial value 0.
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}
```

- [ ] **Step 4: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer::xmodem; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: 14 个测试全部通过。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-core/src/transfer/
git commit -m "feat(core): XMODEM and YMODEM senders"
```

---

### Task 3: ZMODEM 发送端和 `start()`

**Files:**
- Create: `crates/omnyssh-core/src/transfer/zmodem.rs`
- Modify: `crates/omnyssh-core/src/transfer/mod.rs`

- [ ] **Step 1: 先写测试**

在 `transfer/mod.rs` 的 `mod xmodem;` 之后加：

```rust
mod zmodem;
```

新建 `crates/omnyssh-core/src/transfer/zmodem.rs`，**先只放测试模块**。对端用的是 `zmodem2` 自带的 `Receiver`，开了手动接受，这样能模拟 `rz` 跳过已存在的文件（`skip_file()` 发 ZSKIP）。`Peer::round` 每轮先把发送端的输出交给接收端（接收端暂时不收的部分留到下一轮），再把接收端的全部动作处理完。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::test_util::{bytes, drain, file};
    use zmodem2::Receiver;

    /// A `zmodem2` receiver standing in for `rz`.
    struct Peer {
        rx: Receiver,
        /// Names it refuses (ZSKIP), as `rz` does with files that exist.
        skip: Vec<&'static str>,
        pending: Vec<u8>,
        files: Vec<Received>,
        done: bool,
    }

    /// A file as the receiver got it: (name, size, modified, mode) and data.
    type Meta<'a> = (&'a str, Option<u32>, Option<u32>, Option<u32>);
    struct Received {
        name: String,
        size: Option<u32>,
        modified: Option<u32>,
        mode: Option<u32>,
        data: Vec<u8>,
    }

    impl Received {
        fn meta(&self) -> Meta<'_> {
            (&self.name, self.size, self.modified, self.mode)
        }
    }

    impl Peer {
        fn new(skip: &[&'static str]) -> Self {
            let mut rx = Receiver::new().unwrap();
            rx.set_manual_file_accept(true);
            Self {
                rx,
                skip: skip.to_vec(),
                pending: Vec::new(),
                files: Vec::new(),
                done: false,
            }
        }

        /// One exchange each way; false when nothing moved.
        fn round(&mut self, up: &mut dyn Upload) -> bool {
            let out = drain(up);
            let mut moved = !out.is_empty();
            self.pending.extend(out);
            while !self.pending.is_empty() {
                match self.rx.submit_wire(&self.pending) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.pending.drain(..n);
                        moved = true;
                    }
                    Err(e) => panic!("receiver: {e:?}"),
                }
            }
            loop {
                match self.rx.poll() {
                    Action::WriteWire(b) => {
                        let n = b.len();
                        up.receive(b);
                        self.rx.wire_written(n);
                    }
                    Action::WriteFile(b) => {
                        let n = b.len();
                        self.files.last_mut().unwrap().data.extend_from_slice(b);
                        self.rx.file_written(n).unwrap();
                    }
                    Action::Event(Event::FileStarted(info)) => {
                        let name = String::from_utf8(info.name.to_vec()).unwrap();
                        let skip = self.skip.contains(&name.as_str());
                        self.files.push(Received {
                            name,
                            size: info.size.map(|s| s.get()),
                            modified: info.modified,
                            mode: info.mode,
                            data: Vec::new(),
                        });
                        if skip {
                            self.rx.skip_file().unwrap();
                        } else {
                            self.rx.accept_file_at(0).unwrap();
                        }
                    }
                    Action::Event(Event::SessionCompleted) => self.done = true,
                    Action::Event(_) => {}
                    _ => return moved,
                }
                moved = true;
            }
        }

        fn run(&mut self, up: &mut dyn Upload) {
            for _ in 0..100_000 {
                if !self.round(up) {
                    return;
                }
            }
            panic!("the upload did not finish");
        }

        /// Runs until the upload is part-way through a file.
        fn run_until_sending(&mut self, up: &mut dyn Upload) {
            for _ in 0..1000 {
                if let UploadState::Sending { sent, total, .. } = up.state() {
                    if sent > 0 && sent < total {
                        return;
                    }
                }
                self.round(up);
            }
            panic!("never got to sending: {:?}", up.state());
        }
    }

    #[test]
    fn sends_a_batch_with_names_times_and_modes() {
        let a = bytes(200_000, 1);
        let b = bytes(10, 2);
        let files = vec![
            file("a.bin", a.clone()),
            file("b.txt", b.clone()),
            file("empty", vec![]),
        ];
        let mut up = ZmodemUpload::new(files, Instant::now());
        // The shell's echo of `rz -b` and rz's banner come first.
        up.receive(b"rz -b\r\nrz waiting to receive.");
        let mut peer = Peer::new(&[]);
        peer.run(&mut up);
        assert!(peer.done);
        let meta: Vec<_> = peer.files.iter().map(Received::meta).collect();
        let (t, m) = (Some(1_700_000_000), Some(FILE_MODE));
        assert_eq!(
            meta,
            vec![
                ("a.bin", Some(200_000), t, m),
                ("b.txt", Some(10), t, m),
                ("empty", Some(0), t, m)
            ]
        );
        assert_eq!(peer.files[0].data, a);
        assert_eq!(peer.files[1].data, b);
        assert!(peer.files[2].data.is_empty());
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 3,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_file_the_receiver_skips_is_counted_as_skipped() {
        let files = vec![
            file("old.bin", bytes(5000, 3)),
            file("new.bin", bytes(5000, 4)),
        ];
        let mut up = ZmodemUpload::new(files, Instant::now());
        let mut peer = Peer::new(&["old.bin"]);
        peer.run(&mut up);
        assert!(peer.done);
        assert!(peer.files[0].data.is_empty());
        assert_eq!(peer.files[1].data.len(), 5000);
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 1
            }
        );
    }

    #[test]
    fn reports_progress_while_sending() {
        let mut up = ZmodemUpload::new(vec![file("a.bin", bytes(100_000, 5))], Instant::now());
        Peer::new(&[]).run_until_sending(&mut up);
        match up.state() {
            UploadState::Sending {
                file_index,
                file_count,
                file_name,
                total,
                ..
            } => {
                assert_eq!(
                    (file_index, file_count, file_name.as_str(), total),
                    (0, 1, "a.bin", 100_000)
                );
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn bytes_after_the_session_are_leftover() {
        let mut up = ZmodemUpload::new(vec![file("a", vec![1, 2, 3])], Instant::now());
        Peer::new(&[]).run(&mut up);
        assert!(up.state().is_final());
        up.receive(b"$ ");
        // The tail of rz's last header (CR LF) may come first.
        assert!(up.leftover().ends_with(b"$ "));
    }

    #[test]
    fn no_answer_from_rz_fails_and_cancels() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", vec![1])], t);
        let first = drain(&mut up);
        assert!(first.starts_with(b"**\x18B00"), "ZRQINIT first");
        up.tick(t + ZRQINIT_INTERVAL);
        assert_eq!(drain(&mut up), first, "ZRQINIT again");
        up.tick(t + HANDSHAKE_TIMEOUT);
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        match up.state() {
            UploadState::Failed(m) => assert!(m.contains("lrzsz"), "{m}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn cancel_sends_the_cancel_sequence() {
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 6))], Instant::now());
        Peer::new(&[]).run_until_sending(&mut up);
        up.cancel();
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        assert_eq!(up.state(), UploadState::Cancelled);
        up.receive(b"$ ");
        assert_eq!(up.leftover(), b"$ ");
    }

    #[test]
    fn the_receiver_cancelling_fails_the_upload() {
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 7))], Instant::now());
        let mut peer = Peer::new(&[]);
        peer.run_until_sending(&mut up);
        // What `rz` sends on Ctrl+C: CANs, backspaces; then the shell prompt.
        let mut cancel = vec![CAN; 10];
        cancel.extend_from_slice(&[0x08; 10]);
        cancel.extend_from_slice(b"$ ");
        up.receive(&cancel);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device cancelled the transfer".into())
        );
        let mut rest = vec![CAN; 5];
        rest.extend_from_slice(&[0x08; 10]);
        rest.extend_from_slice(b"$ ");
        assert_eq!(up.leftover(), rest, "everything after the fifth CAN");
        assert!(
            !drain(&mut up).contains(&b'*'),
            "no more frames after the cancel"
        );
    }

    #[test]
    fn silence_after_the_handshake_fails() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 8))], t);
        Peer::new(&[]).run_until_sending(&mut up);
        up.tick(t + IDLE_TIMEOUT - Duration::from_millis(1));
        assert!(!up.state().is_final());
        up.tick(t + IDLE_TIMEOUT);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device stopped responding".into())
        );
    }

    #[test]
    fn a_file_over_4_gib_fails_before_anything_is_sent() {
        let mut f = file("huge.img", vec![]);
        f.size = u64::from(u32::MAX) + 1;
        let mut up = ZmodemUpload::new(vec![f], Instant::now());
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        assert_eq!(
            up.state(),
            UploadState::Failed("huge.img is larger than ZMODEM's 4 GiB limit".into())
        );
    }
}
```

- [ ] **Step 2: 确认测试失败**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer::zmodem; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: 编译失败，报 `ZmodemUpload`、`FILE_MODE`、`HANDSHAKE_TIMEOUT` 等找不到。

- [ ] **Step 3: 写实现**

放在 `zmodem.rs` 的最前面（测试模块之前）：

```rust
//! ZMODEM sender for `rz`, wrapping `zmodem2`'s sans-IO `Sender`.
//!
//! What this adds on top: reading the files, the timers (`zmodem2` has none),
//! telling a skipped file from a sent one, and the [`Upload`] states.

use std::io::{Read, SeekFrom};
use std::time::{Duration, Instant};

use zmodem2::{Action, Error, Event, FileInfo, Position, Sender};

use super::{Upload, UploadFile, UploadState, TRANSMIT_CHUNK};

/// How long `rz` gets to answer with ZRINIT after it was started.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often ZRQINIT goes out again while waiting for ZRINIT.
const ZRQINIT_INTERVAL: Duration = Duration::from_secs(2);
/// Silence from the receiver, once it has answered, before giving up. It
/// acknowledges every few subpackets, so this only passes on a dead link.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// A regular file readable by all, writable by its owner: without a mode `rz`
/// creates the file `0600`.
const FILE_MODE: u32 = 0o100_644;
/// ZDLE doubles as CAN. `rz` cancels by sending a run of them; five in a row
/// never occur otherwise (lrzsz's own threshold).
const CAN: u8 = 0x18;
const CANCEL_RUN: u32 = 5;

pub(crate) struct ZmodemUpload {
    sender: Sender,
    files: Vec<UploadFile>,
    index: usize,
    /// `start_file` for `files[index]` is still to be accepted (it refuses
    /// while bytes are queued).
    need_start: bool,
    need_finish: bool,
    /// The current file had data read: a file completed without any was
    /// skipped by the receiver (ZSKIP; `zmodem2` reports it as completed).
    read_any: bool,
    sent: u64,
    skipped: usize,
    /// The receiver has answered (ZRINIT seen, as far as we can tell).
    answered: bool,
    started: Instant,
    last_zrqinit: Instant,
    last_rx: Instant,
    now: Instant,
    inbuf: Vec<u8>,
    /// CANs at the end of what the receiver sent so far.
    cans: u32,
    out: Vec<u8>,
    /// What to report once `abort()`'s cancel sequence is out; `None` means
    /// the receiver cancelled.
    abort_state: Option<UploadState>,
    state: UploadState,
    leftover: Vec<u8>,
}

impl ZmodemUpload {
    pub(crate) fn new(files: Vec<UploadFile>, now: Instant) -> Self {
        let mut up = Self {
            // Only fails when its own buffer cannot hold ZRQINIT.
            sender: Sender::new().expect("ZRQINIT fits"),
            files,
            index: 0,
            need_start: true,
            need_finish: false,
            read_any: false,
            sent: 0,
            skipped: 0,
            answered: false,
            started: now,
            last_zrqinit: now,
            last_rx: now,
            now,
            inbuf: Vec::new(),
            cans: 0,
            out: Vec::new(),
            abort_state: None,
            state: UploadState::Waiting,
            leftover: Vec::new(),
        };
        if let Some(f) = up.files.iter().find(|f| f.size > u64::from(u32::MAX)) {
            let msg = format!("{} is larger than ZMODEM's 4 GiB limit", f.name);
            up.fail(UploadState::Failed(msg));
        }
        up.pump();
        up
    }

    /// Runs the sender until it waits for input or `out` holds a chunk.
    fn pump(&mut self) {
        while self.out.len() < TRANSMIT_CHUNK {
            if !self.state.is_final() {
                self.queue_next();
            }
            match self.sender.poll() {
                Action::WriteWire(bytes) => {
                    let n = bytes.len();
                    self.out.extend_from_slice(bytes);
                    self.sender.wire_written(n);
                }
                Action::ReadFile { offset, max_len } => self.read_file(offset, max_len),
                Action::Event(Event::FileCompleted) => {
                    self.answered = true;
                    if !self.read_any && self.files[self.index].size > 0 {
                        self.skipped += 1;
                    }
                    self.index += 1;
                    self.read_any = false;
                    if self.index < self.files.len() {
                        self.need_start = true;
                    } else {
                        self.need_finish = true;
                    }
                }
                Action::Event(Event::SessionCompleted) => {
                    self.state = UploadState::Done {
                        sent_files: self.files.len() - self.skipped,
                        skipped: self.skipped,
                    };
                    self.leftover.append(&mut self.inbuf);
                }
                Action::Event(Event::Aborted) => {
                    self.state = self.abort_state.take().unwrap_or_else(|| {
                        UploadState::Failed("the device cancelled the transfer".into())
                    });
                    // Unread protocol bytes: noise in a terminal.
                    self.inbuf.clear();
                }
                Action::Event(_) => {}
                _ if self.state.is_final() || self.inbuf.is_empty() => return,
                _ => match self.sender.submit_wire(&self.inbuf) {
                    Ok(0) => return,
                    Ok(n) => {
                        self.inbuf.drain(..n);
                    }
                    // A corrupt header: the sender has queued a ZNAK and
                    // the receiver sends it again.
                    Err(_) => self.inbuf.clear(),
                },
            }
        }
    }

    /// Offers the next file or ends the session when one is due; the sender
    /// refuses both while bytes are queued, so it is retried each round.
    fn queue_next(&mut self) {
        let result = if self.need_start {
            let file = &self.files[self.index];
            let mut info =
                FileInfo::new(file.name.as_bytes(), Some(Position::new(file.size as u32)))
                    .with_mode(FILE_MODE);
            if let Some(t) = file.modified.and_then(|t| u32::try_from(t).ok()) {
                info = info.with_modified(t);
            }
            self.sender
                .start_file(info)
                .map(|()| self.need_start = false)
        } else if self.need_finish {
            self.sender.finish().map(|()| self.need_finish = false)
        } else {
            Ok(())
        };
        match result {
            Ok(()) | Err(Error::Backpressure) => {}
            Err(e) => self.fail(UploadState::Failed(format!("ZMODEM: {e}"))),
        }
    }

    fn read_file(&mut self, offset: Position, max_len: usize) {
        self.answered = true;
        self.read_any = true;
        let file_count = self.files.len();
        let file = &mut self.files[self.index];
        let offset = u64::from(offset.get());
        let want = max_len.min((file.size - offset) as usize);
        let mut buf = vec![0u8; want];
        let read = file
            .reader
            .seek(SeekFrom::Start(offset))
            .and_then(|_| file.reader.read_exact(&mut buf));
        if let Err(e) = read {
            let msg = if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "the file got shorter while it was being sent".to_string()
            } else {
                e.to_string()
            };
            let msg = format!("could not read {}: {msg}", file.name);
            self.fail(UploadState::Failed(msg));
            return;
        }
        self.sent = offset + want as u64;
        self.state = UploadState::Sending {
            file_index: self.index,
            file_count,
            file_name: file.name.clone(),
            sent: self.sent,
            total: file.size,
        };
        if let Err(e) = self.sender.submit_file(&buf) {
            self.fail(UploadState::Failed(format!("ZMODEM: {e}")));
        }
    }

    /// Drops what is queued and sends the cancel sequence; `state` is
    /// reported once it is out.
    fn fail(&mut self, state: UploadState) {
        self.out.clear();
        self.abort_state = Some(state);
        self.need_start = false;
        self.need_finish = false;
        self.sender.abort();
    }

    fn finishing(&self) -> bool {
        self.state.is_final() || self.abort_state.is_some()
    }
}

impl Upload for ZmodemUpload {
    fn receive(&mut self, data: &[u8]) {
        if self.state.is_final() {
            self.leftover.extend_from_slice(data);
            return;
        }
        self.last_rx = self.now;
        for (i, &b) in data.iter().enumerate() {
            self.cans = if b == CAN { self.cans + 1 } else { 0 };
            if self.cans >= CANCEL_RUN && !self.finishing() {
                // `zmodem2` only knows the ZABORT/ZCAN headers.
                self.inbuf.clear();
                self.leftover.extend_from_slice(&data[i + 1..]);
                self.fail(UploadState::Failed(
                    "the device cancelled the transfer".into(),
                ));
                self.pump();
                return;
            }
        }
        self.inbuf.extend_from_slice(data);
        self.pump();
    }

    fn transmit(&mut self, out: &mut Vec<u8>) {
        self.pump();
        let n = self.out.len().min(TRANSMIT_CHUNK);
        out.extend(self.out.drain(..n));
    }

    fn tick(&mut self, now: Instant) {
        self.now = now;
        if self.finishing() {
            return;
        }
        if !self.answered {
            if now.duration_since(self.started) >= HANDSHAKE_TIMEOUT {
                self.fail(UploadState::Failed(
                    "the device did not answer rz (is lrzsz installed?)".into(),
                ));
            } else if now.duration_since(self.last_zrqinit) >= ZRQINIT_INTERVAL {
                self.last_zrqinit = now;
                let _ = self.sender.timeout();
            }
        } else if now.duration_since(self.last_rx) >= IDLE_TIMEOUT {
            self.fail(UploadState::Failed("the device stopped responding".into()));
        }
        self.pump();
    }

    fn cancel(&mut self) {
        if !self.finishing() {
            self.fail(UploadState::Cancelled);
            self.pump();
        }
    }

    fn state(&self) -> UploadState {
        self.state.clone()
    }

    fn leftover(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.leftover)
    }
}
```

- [ ] **Step 4: 加 `start()`**

在 `transfer/mod.rs` 里，`pub trait Upload { … }` 之后、`#[cfg(test)] pub(crate) mod test_util` 之前插入：

```rust
/// Starts an upload. `files` must not be empty, and XMODEM takes exactly one
/// (see [`Protocol::batch`]); callers check both first.
pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Box<dyn Upload> {
    assert!(!files.is_empty(), "an upload needs a file");
    match protocol {
        Protocol::Zmodem => Box::new(zmodem::ZmodemUpload::new(files, now)),
        Protocol::Ymodem => Box::new(xmodem::XmodemUpload::new(
            xmodem::Variant::Ymodem,
            files,
            now,
        )),
        Protocol::Xmodem => {
            assert!(files.len() == 1, "XMODEM sends one file");
            Box::new(xmodem::XmodemUpload::new(
                xmodem::Variant::Xmodem,
                files,
                now,
            ))
        }
    }
}
```

- [ ] **Step 5: 跑测试**

```bash
nohup sh -c 'cargo test -p omnyssh-core --lib transfer; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: `transfer` 下 26 个测试全部通过（mod 3 个、xmodem 14 个、zmodem 9 个）。

- [ ] **Step 6: Commit**

```bash
git add crates/omnyssh-core/src/transfer/
git commit -m "feat(core): ZMODEM sender over zmodem2"
```

---

### Task 4: 和 lrzsz 的互通测试

**Files:**
- Create: `crates/omnyssh-core/tests/transfer_lrzsz.rs`

- [ ] **Step 1: 准备 lrzsz（不需要 root）**

先 `which rz rb rx`。没有的话，在 Debian/Ubuntu 上解包到临时目录：

```bash
mkdir -p /tmp/lrzsz && cd /tmp/lrzsz && apt-get download lrzsz && dpkg-deb -x lrzsz_*.deb . && ls usr/bin
```

Expected: 列出 `rb rx rz sb sx sz`。之后用 `LRZSZ_DIR=/tmp/lrzsz/usr/bin` 指向它们。

- [ ] **Step 2: 写测试**

```rust
//! Uploads to the real lrzsz receivers (`rz`, `rb`, `rx`) over pipes.
//!
//! Ignored by default: they need lrzsz. Run with
//! `cargo test -p omnyssh-core --test transfer_lrzsz -- --ignored`; set
//! `LRZSZ_DIR` to the directory holding the binaries when they are not on
//! `PATH`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use omnyssh_core::transfer::{start, Protocol, UploadFile, UploadState};

fn bin(name: &str) -> PathBuf {
    match std::env::var_os("LRZSZ_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

fn random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Runs `receiver` in `dir` and uploads `paths` to it; returns the final state.
fn upload(
    receiver: &str,
    args: &[&str],
    dir: &Path,
    protocol: Protocol,
    paths: &[PathBuf],
) -> UploadState {
    let mut child = Command::new(bin(receiver))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("could not start {receiver} (is lrzsz installed?): {e}"));
    let mut to_rx = child.stdin.take().unwrap();
    let mut from_rx = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = from_rx.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let files = paths.iter().map(|p| UploadFile::open(p).unwrap()).collect();
    let mut up = start(protocol, files, Instant::now());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "timed out in {:?}", up.state());
        let mut out = Vec::new();
        up.transmit(&mut out);
        if !out.is_empty() {
            to_rx.write_all(&out).unwrap();
            to_rx.flush().unwrap();
            continue;
        }
        if up.state().is_final() {
            break;
        }
        if let Ok(data) = rx.recv_timeout(Duration::from_millis(20)) {
            up.receive(&data);
        }
        up.tick(Instant::now());
    }
    drop(to_rx);
    let status = child.wait().unwrap();
    let state = up.state();
    if matches!(state, UploadState::Done { .. }) {
        assert!(status.success(), "{receiver} exited with {status}");
    }
    state
}

fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

#[test]
#[ignore = "needs lrzsz"]
fn zmodem_to_rz() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let big = random(1 << 20, 1);
    let paths = vec![
        write(src.path(), "big.bin", &big),
        write(src.path(), "small.txt", b"hello\n"),
        write(src.path(), "empty", b""),
    ];
    let state = upload("rz", &["-b"], dst.path(), Protocol::Zmodem, &paths);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 3,
            skipped: 0
        }
    );
    assert_eq!(std::fs::read(dst.path().join("big.bin")).unwrap(), big);
    assert_eq!(
        std::fs::read(dst.path().join("small.txt")).unwrap(),
        b"hello\n"
    );
    assert!(std::fs::read(dst.path().join("empty")).unwrap().is_empty());

    // Again: rz skips files that already exist.
    let state = upload("rz", &["-b"], dst.path(), Protocol::Zmodem, &paths[..2]);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 0,
            skipped: 2
        }
    );
}

#[test]
#[ignore = "needs lrzsz"]
fn ymodem_to_rb() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let big = random((1 << 20) + 77, 2);
    let paths = vec![
        write(src.path(), "fw.bin", &big),
        write(src.path(), "b.txt", b"hi"),
    ];
    let state = upload("rb", &["-b"], dst.path(), Protocol::Ymodem, &paths);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 2,
            skipped: 0
        }
    );
    // YMODEM's block 0 carries the size, so the padding is cut off.
    assert_eq!(std::fs::read(dst.path().join("fw.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dst.path().join("b.txt")).unwrap(), b"hi");
}

#[test]
#[ignore = "needs lrzsz"]
fn xmodem_to_rx_with_crc_and_with_checksum() {
    for args in [&["-b", "-c", "out.bin"][..], &["-b", "out.bin"][..]] {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        // A whole number of 128-byte blocks: XMODEM pads the last one.
        let data = random(128 * 1000, 3);
        let paths = vec![write(src.path(), "in.bin", &data)];
        let state = upload("rx", args, dst.path(), Protocol::Xmodem, &paths);
        assert_eq!(
            state,
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            },
            "rx {args:?}"
        );
        assert_eq!(
            std::fs::read(dst.path().join("out.bin")).unwrap(),
            data,
            "rx {args:?}"
        );
    }
}
```

- [ ] **Step 3: 跑互通测试**

```bash
nohup sh -c 'LRZSZ_DIR=/tmp/lrzsz/usr/bin cargo test -p omnyssh-core --test transfer_lrzsz -- --ignored; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

（本机 PATH 里已有 lrzsz 时去掉 `LRZSZ_DIR=…`。）

Expected: `zmodem_to_rz`、`ymodem_to_rb`、`xmodem_to_rx_with_crc_and_with_checksum` 3 个测试通过，用时 5 秒左右。不加 `--ignored` 时这 3 个显示为 ignored。

- [ ] **Step 4: Commit**

```bash
git add crates/omnyssh-core/tests/transfer_lrzsz.rs
git commit -m "test(core): uploads to the real rz, rb and rx"
```

---

### Task 5: 全量检查

- [ ] **Step 1: 跑全部检查**

```bash
nohup sh -c 'cargo test -p omnyssh-core 2>&1 | grep -E "FAILED|panicked|test result"; cargo clippy -p omnyssh-core --all-targets -- -D warnings 2>&1 | tail -3; cargo fmt --check -p omnyssh-core && echo FMT_OK; echo EXIT=$?' > /tmp/o.log 2>&1 &
```
然后反复 `tail -5 /tmp/o.log`，直到出现 `EXIT=`。

Expected: 每个 `test result` 都是 `ok`，没有 FAILED；clippy 以 `Finished` 结束，没有 warning 或 error；输出 `FMT_OK`。

- [ ] **Step 2: 如果 fmt 有改动**

运行 `cargo fmt -p omnyssh-core`，再跑一次 Step 1，然后：

```bash
git add -u crates/omnyssh-core
git commit -m "style(core): rustfmt the transfer module"
```

没有改动就跳过这一步。
