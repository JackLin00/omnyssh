# 拖放上传文件 — 设计

日期：2026-11-07　分支：`feat/serial-port`

## 目标
把本地文件拖到 SSH 终端窗格或串口标签页上，松手就把文件传到设备上，体验类似 Xshell。要能用于三类设备：装了 `rz`（lrzsz）的 Linux、bootloader（U-Boot `loady`/`loadx`、MCU）、可以走 SFTP 的 SSH 主机。

## 已与用户确认
- 协议**按会话预设**，拖进去直接开始传，不弹出选择菜单。
- SSH 可以选 ZMODEM 或 SFTP。SFTP 传到预设目录，不跟随 shell 的当前目录。
- 串口可以选 ZMODEM、YMODEM 或 XMODEM。
- 采用方案 A：传输引擎放在 `omnyssh-core`，按 sans-IO 方式接进现有的串口读写循环和 SSH pump 循环。

## 配置
- `Host`（`omnyssh-core/src/ssh/client.rs`）新增两个字段：
  - `upload_protocol`：`zmodem | sftp`，默认 `zmodem`。值为默认时不序列化。
  - `upload_dir`：`Option<String>`，只对 SFTP 生效，没设置时等于 `~`。
- `SerialDevice`（`omnyssh-core/src/config/serial_devices`）新增 `upload_protocol`：`zmodem | ymodem | xmodem`，默认 `zmodem`。值为默认时不序列化。
- XMODEM 和 YMODEM 不加任何配置项：
  - 都优先用 CRC-16 和 1K 块。
  - XMODEM 的接收端如果回 NAK 而不是 `C`，就按规范降级为 128 字节块加校验和。
- 主机编辑表单和串口设备表单里，各加一个"拖放上传协议"下拉框。SFTP 时再加一个"上传目录"输入框。
- 会话工具栏有一个协议下拉框，用来**临时覆盖**协议，只对本次会话生效，不写回配置。没有保存的临时串口会话默认用 ZMODEM。

## 拖放交互
- 用 Tauri 2 webview 的 `onDragDropEvent`，按坐标找到落点所在的终端窗格或串口标签页。落点不在任何终端上就不做任何事。
- 拖动经过时，目标窗格上显示半透明遮罩："松开以 <协议> 上传到 <会话名>"。
- **多个文件**：ZMODEM、YMODEM、SFTP 都支持一次传多个。XMODEM 拖入多个文件时拒绝，提示"XMODEM 仅支持单文件"。
- **文件夹**：拒绝，提示"不支持上传文件夹"。
- **已有传输进行中**：拒绝，提示"已有传输进行中"。
- **传输期间**（ZMODEM、YMODEM、XMODEM）：
  - 窗格底部显示进度条：文件名、`第 i/n 个`、百分比、速率，以及"取消"按钮。
  - 终端的键盘输入被禁用，避免按键混进协议数据流。
  - 结束后在终端里打印一行，比如 `[上传完成] foo.bin 1.2 MB`、`[上传失败] 原因`，有跳过的文件时还会有 `[上传完成] 3 个文件，已跳过 1 个`。
- **SFTP**：终端不受影响，不禁用输入。进度用同一个进度条显示，数据来自现有的 `transfer-progress` 事件。

### 各协议怎么启动
- **ZMODEM**：自动向会话写入 `rz -be\r`（`-b` 二进制，`-e` 转义控制字符），然后等对方回 `ZRINIT`。
- **YMODEM / XMODEM**：什么都不发，等接收端发起（`C` 或 NAK）。用户需要先在设备上执行 `loady`、`loadx`、`rb`、`rx` 这类命令。等待期间进度条显示"等待接收端…"。
- **SFTP**：上传到 `upload_dir/<文件名>`。`~` 和 `~/x` 转换为 SFTP 的相对路径 `.` 和 `x`，SFTP 的相对路径以 home 为基准。

## 架构

### core：`omnyssh-core/src/transfer/`
- `mod.rs`：定义接口、`Protocol` 和 `start()`。
- `xmodem.rs`：XMODEM 和 YMODEM 共用分块、CRC 和 ACK 状态机。YMODEM 多一个 0 号块（文件名 NUL 大小）和批量结束的空 0 号块。
- `zmodem.rs`：只实现发送方向。实现计划的第一步是 spike `zmodem2` crate：它能和真实的 `rz` 互通，就把它包在接口后面；不能，就自己实现 ZMODEM 发送端。对外接口都一样。

```rust
pub enum Protocol { Zmodem, Ymodem, Xmodem }

pub struct UploadFile {
    pub name: String,
    pub size: u64,
    pub reader: Box<dyn Read + Send>,
}

pub enum UploadState {
    Waiting,
    Sending { file_index: usize, file_count: usize, file_name: String, sent: u64, total: u64 },
    Done { sent_files: usize, skipped: usize },
    Failed(String),
    Cancelled,
}

pub trait Upload: Send {
    /// 设备发回的字节。
    fn receive(&mut self, data: &[u8]);
    /// 追加待发送的字节。调用方只在自己的发送缓冲排空后才调用，形成背压。
    fn transmit(&mut self, out: &mut Vec<u8>);
    /// 处理超时和重传，时间由调用方注入。
    fn tick(&mut self, now: Instant);
    /// 发出取消序列：ZMODEM 发 8 个 CAN 加 8 个退格，X/YMODEM 发 CAN CAN。
    fn cancel(&mut self);
    fn state(&self) -> UploadState;
    /// 结束后多收到的字节（比如 ZMODEM 的 "OO"、shell 提示符），交还给终端显示。
    fn leftover(&mut self) -> Vec<u8>;
}

pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Box<dyn Upload>;
```

- 引擎不碰端口和系统时钟。文件在引擎内部按块同步读取（128 B 到 8 KB）。

### 串口接入：`serial.rs` 的 `run()`
- `Ctrl` 新增 `Upload(Box<dyn Upload>)` 和 `CancelUpload`。`SerialOutput` 新增 `Upload(UploadState)`。
- 有传输进行时，每轮循环：
  1. 读到的字节交给 `upload.receive`，**不调用** `on_output(Data)`；
  2. `pending` 为空时才调用 `upload.transmit(&mut pending)`；
  3. 调用 `upload.tick(Instant::now())`；
  4. 状态和上一次不同时，调用 `on_output(SerialOutput::Upload(state))`。
- 进入 `Done`、`Failed` 或 `Cancelled` 后，把 `leftover()` 作为普通的 `Data` 交还，然后回到正常模式。
- 现有的"有界写、不 flush"策略不变。
- 传输期间收到的 `Ctrl::Write` 照常进入 `pending`。GUI 已经禁用了输入，这条路径只用于 ZMODEM 启动前写入 `rz -be\r`。

### SSH 接入：`ssh/pty.rs` 的 pump 循环
- `Ctrl` 新增 `Upload(Box<dyn Upload>)` 和 `CancelUpload`。`CoreEvent` 新增 `UploadState(SessionId, UploadState)`。
- `select!` 里加一个 100 ms 的 `interval` 分支，只在传输中启用，用来调用 `tick`。
- 传输中，`ChannelMsg::Data` 和 `ExtendedData` 交给 `upload.receive`，不喂给 vt100 parser，也不发往 `raw_output`。
- 每次 `receive` 和 `tick` 之后，把 `transmit` 的输出交给 `channel.data(..).await`，russh 的窗口提供背压。
- 结束时，`leftover()` 按普通输出处理：喂给 parser，发往 `raw_output`。
- 传输模式下的字节路由抽成一个能单测的小函数。TUI 不会发 `Ctrl::Upload`，所以行为不变。

### GUI：`omnyssh-gui`
- 新增命令，都放在新文件 `commands/upload.rs`：
  - `terminal_upload(session_id, protocol, paths)`
  - `serial_upload(session_id, protocol, paths)`
  - `upload_cancel(session_id)`
  - 协议由前端传入，因为工具栏的临时覆盖保存在前端。
- 命令负责：
  - 检查路径（文件夹、XMODEM 多文件、会话已有传输进行中都直接返回错误）；
  - 打开文件，取文件名和大小；
  - ZMODEM 时先写入 `rz -be\r`，再发 `Ctrl::Upload`。
- SFTP 协议走 `terminal_upload`，分支到现有的 `SftpManager`：
  - 第一次拖放时，在后台为这个主机开一个 SFTP 会话并缓存，SSH 标签关闭时一起关掉；
  - 每个文件依次调用现有的上传路径。
- 新增事件：
  - `upload-progress { sessionId, fileIndex, fileCount, fileName, sent, total }`，按会话节流到每 100 ms 最多一次；
  - `upload-finished { sessionId, ok, message }`。
  - SFTP 的 `transfer-progress` 在 bridge 里按 transfer id 映射回会话，转成 `upload-progress`，前端只需要处理一种事件。
- **会话日志**：协议字节不写入日志，结束时写一行 `[upload] <文件名> <大小> ok|failed: <原因>`。

### 前端：`omnyssh-gui/ui`
- 拖放的命中判断和协议决策（预设、临时覆盖、多文件和文件夹拒绝）放进一个纯 TS 模块。
- `TerminalPane.svelte` 和 `SerialView.svelte` 负责遮罩、进度条，以及传输期间禁用输入。
- 主机表单和串口设备表单加协议字段，SFTP 时加上传目录。

## 错误与取消

| 情况 | 处理 |
|---|---|
| ZMODEM 发出 `rz` 后 10 秒没收到 `ZRINIT` | 发取消序列，报失败："设备未响应 rz，确认已安装 lrzsz" |
| X/YMODEM 等接收端超过 60 秒 | 报失败："接收端未就绪（是否已执行 loady/loadx？）" |
| 同一个块重传超过 10 次 | 发 CAN，报失败："重传过多（块 N）" |
| 对端发来 CAN CAN 或 `ZABORT`/`ZCAN` | 报失败："设备取消了传输" |
| 用户点取消 | 引擎发取消序列，最多等 2 秒排空发送缓冲，然后报"已取消" |
| 本地读文件出错 | 发 CAN，报失败，附上 I/O 错误 |
| 传输中串口拔出或 SSH 断开 | 传输随会话结束，`upload-finished` 报"连接中断"。串口重连后不会续传 |
| ZMODEM 收到 `ZSKIP` | 这个文件记为跳过，继续下一个，最终结果里写跳过的数量 |
| SFTP 失败 | 沿用现有的错误信息，通过 `upload-finished` 报告 |

所有结束情况都会退出传输模式、恢复键盘输入，并在终端里打印一行结果。

## 测试
- **引擎单元测试**（`transfer/*`，纯内存，用 `Cursor` 当文件，时间手动推进）：
  - X/YMODEM：
    - 用手写的接收端模拟器跑完整流程，逐块比对字节；
    - 覆盖 CRC 和校验和降级、NAK 重传、超时、重传上限、对端 CAN、用户取消；
    - YMODEM 的 0 号块格式、批量结束块、最后一块的 `0x1A` 填充。
  - ZMODEM：
    - 帧编解码和 ZDLE 转义；
    - 用最小接收端模拟器覆盖握手、多文件、`ZSKIP`、取消、`ZRINIT` 超时。
- **互通测试**（`omnyssh-core/tests/transfer_lrzsz.rs`，用 `#[ignore]` 标记，本机有 lrzsz 时运行）：
  - 通过 pty 对接真实的 `rz`、`rb`（YMODEM）、`rx`（XMODEM），各上传一个随机 1 MB 文件，比对 SHA-256；
  - 它同时是 `zmodem2` spike 的验收标准。
- **会话接入测试**：
  - 串口：沿用假端口，验证传输期间数据不进 `on_output(Data)`、leftover 能回到终端，以及取消和拔出路径；
  - SSH：测试抽出来的字节路由函数。
- **前端**：
  - Vitest：命中判断、协议决策；
  - Playwright e2e：用 stub 的 `__TAURI_INTERNALS__` 模拟拖放事件和 `upload-progress`、`upload-finished`，验证遮罩、进度条、输入禁用和取消。

## 不在本次范围内
- 下载：`sz`，以及设备主动发起 ZMODEM 时的自动检测。
- 断点续传。
- 上传文件夹。
- 没有 `rz` 时的 base64 等替代方案。
- TUI 端的支持。
