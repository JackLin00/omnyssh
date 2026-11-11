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
  - 结束后在终端里打印一行（英文，界面文字都是英文），比如 `[upload] uploaded foo.bin (1.2 MB)`、`[upload] failed: <原因>`、`[upload] uploaded 2 of 3 files, 1 skipped by the receiver`。完整规则见计划 P 决定 2。
- **SFTP**：终端不受影响，不禁用输入。进度用同一个进度条显示，同样来自 `upload-progress` 事件。

### 各协议怎么启动
- **ZMODEM**：自动向会话写入 `rz -b\r`（`-b` 二进制），然后等 `rz` 主动发来的 `ZRINIT`；我们不发 ZRQINIT（没装 `rz` 时 shell 会把它回显回来，造成死循环）。不加 `-e`：`zmodem2` 不支持"转义全部控制字符"（spike 中 `rz -be` 握手失败）；ZMODEM 标准转义已覆盖 ZDLE、DLE、XON、XOFF，软件流控的串口也安全。
- **YMODEM / XMODEM**：什么都不发，等接收端发起（`C` 或 NAK）。用户需要先在设备上执行 `loady`、`loadx`、`rb`、`rx` 这类命令。等待期间进度条显示"等待接收端…"。
- **SFTP**：上传到 `upload_dir/<文件名>`。`upload_dir` 未设置或为 `~` 时目标就是文件名本身，`~/x` 变成 `x/文件名`（SFTP 的相对路径以 home 为基准）。

## 架构

### core：`omnyssh-core/src/transfer/`
- `mod.rs`：定义接口、`Protocol` 和 `start()`。
- `xmodem.rs`：XMODEM 和 YMODEM 共用分块、CRC 和 ACK 状态机。YMODEM 多一个 0 号块（文件名 NUL 大小）和批量结束的空 0 号块。
- `zmodem.rs`：只实现发送方向，包装 `zmodem2` 0.7.3 的 `Sender`（sans-IO，`poll()` 返回 `WriteWire`/`ReadFile`/`Event`/`Idle`）。
  - 2026-11-07 spike 已验证：和 lrzsz 0.12.21 的 `rz -b` 经管道互传 1 MB 随机文件 + 小文件 + 空文件，SHA-256 一致，用时约 0.17 s；目标文件已存在时 `rz` 回 `ZSKIP`，引擎照常继续下一个。
  - `FileInfo` 必须带 `with_modified(本地 mtime)` 和 `with_mode(0o100644)`：不带时 `rz` 会把文件时间写成乱值（spike 中出现 2446 年），权限变成 `0600`。
  - `zmodem2` 对 `ZSKIP` 只发 `FileCompleted`，不单独报告。判断方法：`FileCompleted` 时这个文件一个数据块都没读过，而且文件大小大于 0，就记为跳过。

```rust
pub enum Protocol { Zmodem, Ymodem, Xmodem }

/// Read + Seek + Send：ZMODEM 会按接收端的 ZRPOS 回到某个偏移重发。
pub trait Source: Read + Seek + Send {}

pub struct UploadFile {
    pub name: String,
    pub size: u64,
    /// 秒（Unix 时间），ZMODEM 和 YMODEM 会发给接收端。
    pub modified: Option<u64>,
    pub reader: Box<dyn Source>,
}

pub enum UploadState {
    Waiting,
    Sending { file_index: usize, file_count: usize, file_name: String, sent: u64, total: u64 },
    Done { sent_files: usize, skipped: usize },
    Failed(String),
    Cancelled,
}

pub trait Upload: Send {
    /// 设备发回的字节。调用前先 tick(Instant::now())：由收到的字节启动的计时用的是最近一次 tick 的时间。
    fn receive(&mut self, data: &[u8]);
    /// 追加待发送的字节。调用方只在自己的发送缓冲排空后才调用，形成背压。
    fn transmit(&mut self, out: &mut Vec<u8>);
    /// 处理超时和重传，时间由调用方注入。
    fn tick(&mut self, now: Instant);
    /// 发出取消序列：三种协议都发 10 个 CAN 加 10 个退格（和 lrzsz 的 canit() 相同；U-Boot 要 3 个 CAN 才取消）。
    fn cancel(&mut self);
    fn state(&self) -> UploadState;
    /// 结束后多收到的字节（`rz` 最后一帧尾部的 CR LF、shell 提示符；没装 `rz` 时还包括握手前 shell 的输出，最多 4 KiB），交还给终端显示。
    fn leftover(&mut self) -> Vec<u8>;
}

pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Box<dyn Upload>;
```

- 引擎不碰端口和系统时钟。文件在引擎内部按块同步读取，每次最多 1 KiB；每次 `transmit` 最多交出 8 KiB。读的是本地文件，在 tokio 任务里同步读也可以接受。完整接口以 `crates/omnyssh-core/src/transfer/mod.rs` 为准。

### 串口接入：`serial.rs` 的 `run()`
- `Ctrl` 新增 `Upload { preamble: Vec<u8>, upload: ActiveUpload }` 和 `CancelUpload`。收到 `Upload` 时，先把 `preamble`（ZMODEM 时是 `rz -b\r`，X/YMODEM 时为空）放进 `pending`，同时进入传输模式，这样此后读到的每个字节都交给引擎。`SerialOutput` 新增 `Upload(UploadEvent)`（`Progress(UploadState)` 或 `Finished(UploadResult)`）。传输的驱动逻辑（节流、排空、结果行）在串口和 SSH 共用的 `transfer::ActiveUpload` 里。
- 有传输进行时，每轮循环：
  1. 读到的字节交给 `upload.receive`，**不调用** `on_output(Data)`；
  2. 调用 `upload.tick(now)`，再调用 `upload.pump(now, &mut pending)`：`pending` 为空时由它补充待发字节；
  3. 进度经 `ActiveUpload::progress` 节流到 100 ms，作为 `Upload(Progress)` 报告。
- 结束且排空后（最多等 2 s），把 leftover 加结果行作为一个 `Data` 交还，再发 `Upload(Finished)`，然后回到正常模式。
- 关闭端口时如果有上传在进行，先取消，取消序列随关闭前的排空发出。
- 现有的"有界写、不 flush"策略不变。
- 传输期间收到的 `Ctrl::Write` 照常进入 `pending`。GUI 已经禁用了输入，正常情况下不会有。

### SSH 接入：`ssh/pty.rs` 的 pump 循环
- `Ctrl` 新增 `Upload { preamble, upload }`（同串口）和 `CancelUpload`。收到 `Upload` 时，在同一个 `select!` 分支里先 `channel.data(preamble)`，再进入传输模式。`tokio::select!` 在多个分支同时就绪时随机选，所以如果把写入 `rz -b\r` 和 `Ctrl::Upload` 拆成两条消息，中间可能先处理一次 `channel.wait()`，把 ZRINIT 当成普通输出。`CoreEvent` 新增 `Upload(SessionId, UploadEvent)`。
- `select!` 里加一个 100 ms 的 `interval` 分支（创建一次，`MissedTickBehavior::Skip`，用 `if upload.is_some()` 守卫），只在传输中启用，用来调用 `tick`。每次 `receive` 之前也先 `tick(Instant::now())`。
- 传输中，`ChannelMsg::Data` 和 `ExtendedData` 交给 `upload.receive`，不喂给 vt100 parser，也不发往 `raw_output`。
- 每次 `receive` 和 `tick` 之后，把 `transmit` 的输出交给 `channel.data(..).await`，russh 的窗口提供背压。
- 结束时，`leftover()` 按普通输出处理：喂给 parser，发往 `raw_output`。
- 由 `crates/omnyssh-core/tests/upload_pty.rs` 经进程内 SSH 服务端做端到端测试（含经真实 `rz` 的用例）。关闭标签时如果有上传在进行，先发取消序列（最多等 500 ms）。TUI 不会发 `Ctrl::Upload`，所以行为不变。

### GUI：`omnyssh-gui`
- 新增命令，都放在新文件 `commands/upload.rs`：
  - `terminal_upload(session_id, protocol, paths)`
  - `serial_upload(session_id, protocol, paths)`
  - `upload_cancel(session_id)`
  - 协议由前端传入，因为工具栏的临时覆盖保存在前端。
- 命令负责：
  - 检查路径（文件夹、XMODEM 多文件、会话已有传输进行中都直接返回错误）；
  - 打开文件，取文件名和大小；
  - 发 `Ctrl::Upload { preamble, upload }`：ZMODEM 时 `preamble` 是 `rz -b\r`，X/YMODEM 时为空。
- SFTP 协议走 `terminal_upload`，分支到现有的 `SftpManager`：
  - 每个终端第一次 SFTP 拖放时，为该终端开一条专用 SFTP 连接（用终端打开时的主机快照），终端关闭或远端退出时关掉；
  - 每个文件依次调用现有的上传路径。
- 新增事件：
  - `upload-progress { sessionId, waiting, fileIndex, fileCount, fileName, sent, total }`，每个会话最多 100 ms 一次（协议上传在 core 的 `ActiveUpload` 里节流，SFTP 在 GUI 里按文件节流）；
  - `upload-finished { sessionId, ok, message }`。
  - 专用 SFTP 连接的 `FileTransferProgress` 由 `forward_upload_sftp_events` 直接转成 `upload-progress`，不使用 `transfer-progress`，前端只需要处理一种事件。
- **会话日志**：协议字节不写入日志，终端里的结果行（见计划 P 决定 2）作为普通输出进入日志。

### 前端：`omnyssh-gui/ui`
- 拖放的命中判断和协议决策（预设、临时覆盖、多文件和文件夹拒绝）放进一个纯 TS 模块。
- `TerminalPane.svelte` 和 `SerialView.svelte` 负责遮罩、进度条，以及传输期间禁用输入。
- 主机表单和串口设备表单加协议字段，SFTP 时加上传目录。

## 错误与取消

| 情况 | 处理 |
|---|---|
| ZMODEM 发出 `rz` 后 25 秒没收到 `ZRINIT`（覆盖 `rz` 两次约 10 秒一次的重发） | 发取消序列，报失败："设备未响应 rz，确认已安装 lrzsz" |
| X/YMODEM 等接收端超过 60 秒 | 报失败："接收端未就绪（是否已执行 loady/loadx？）" |
| 同一个块重传超过 10 次 | 发 CAN，报失败："重传过多（块 N）" |
| 对端发来 CAN CAN 或 `ZABORT`/`ZCAN` | 报失败："设备取消了传输" |
| 用户点取消 | 引擎发取消序列，最多等 2 秒排空发送缓冲，然后报 `cancelled`。ZMODEM 先丢掉缓冲里还没发出的文件数据。SFTP 在当前文件传完后生效；连接建立期间取消则 `terminal_upload` 返回 `Err("cancelled")` |
| 本地读文件出错 | 发 CAN，报失败，附上 I/O 错误 |
| 传输中串口拔出或 SSH 断开 | 协议上传报 `the connection was lost`；SFTP 上传随终端结束，不报 `upload-finished`（前端以 `terminal-exited` 结束）。串口重连后不会续传 |
| ZMODEM 收到 `ZSKIP` | 这个文件记为跳过（判断方法见上），继续下一个，最终结果里写跳过的数量 |
| SFTP 失败 | 报 `<文件名>: <错误>`，并丢弃这条连接（下次拖放重连）；连接断开报 `the SFTP connection was lost` |

所有结束情况都会退出传输模式、恢复键盘输入，并在终端里打印一行结果。例外：标签被关闭、登录中被拒的上传、SFTP 连接建立期间取消、SFTP 上传时远端退出，这些情况没有结果行。

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
  - 通过管道（stdin/stdout）对接真实的 `rz`、`rb`（YMODEM）、`rx`（XMODEM），上传约 1 MB 的随机文件，逐字节比对；带 60 s 看门狗，卡住时会失败而不是挂住；
  - ZMODEM 的这一项在 spike 中已手动验证通过，这里把它固化为测试。
- **会话接入测试**：
  - 串口：沿用假端口，验证传输期间数据不进 `on_output(Data)`、leftover 能回到终端，以及取消和拔出路径；
  - SSH：测试抽出来的字节路由函数。
- **前端**：
  - Vitest：命中判断、协议决策；
  - Playwright e2e：用 stub 的 `__TAURI_INTERNALS__` 模拟拖放事件和 `upload-progress`、`upload-finished`，验证遮罩、进度条、输入禁用和取消。

## 接入时的注意事项（引擎实现与审查中发现，供接入与界面计划使用）
- 写入 `rz -b\r` 和进入传输模式必须是一步：用 `Ctrl::Upload { preamble, upload }` 一条消息完成（见上面串口和 SSH 接入）。否则 `rz` 的第一个 ZRINIT 可能被当成普通输出显示到终端里。
- GUI 在发 `Ctrl::Upload` **之前**就要检查文件：文件夹（在调用 `UploadFile::open` 之前检查；Windows 上打开文件夹报的是"拒绝访问"）、空列表、XMODEM 多文件、大于 4 GiB（ZMODEM）、文件名超过 256 字节（ZMODEM、YMODEM）、不是普通文件（FIFO、设备文件），都直接拒绝。否则 `start()` 会返回一个已经失败的上传，并向 shell 发取消序列；空列表或 XMODEM 多文件会让 `start()` panic。
- 多文件 YMODEM 发给 U-Boot `loady`：U-Boot 只收第一个文件。界面上拖入多个文件且协议是 YMODEM 时，给出提示。
- ZMODEM 的"跳过"不一定是"文件已存在"，`rz` 无法创建文件时也会跳过。界面文字写"已跳过"，不要写"已存在"。
- 进度可能倒退（ZRPOS 回退重发）；跳过或空文件之后，状态会继续显示上一个文件的 100%，直到下一个文件开始读数据。
- ZMODEM 在 60 s 内没有进度就判定卡住，最低支持 2400 波特率；X/YMODEM 是同一块 11 次、每次 10 s 没有应答才放弃（约 110 s）。
- 取消或对端取消之后，leftover 开头可能有 CAN 和退格字节；在终端里打印结果行之前先输出 `\r\n`。
- 退出传输模式：一直把收到的字节交给 `receive`，直到 `state().is_final()` 并且 `transmit` 不再交出字节（ZMODEM 在 `Done` 之后还有 `OO` 要发），然后取 `leftover()`。取消时最多等 2 s 排空，这一步由会话负责。
- 传输中连接断开（串口拔出、SSH 断开），引擎没有对应的接口，由会话自己生成失败结果，消息用英文（如 `the connection was lost`），和引擎的消息风格一致。
- 主机配置的协议枚举是 `{Zmodem, Sftp}`，和 `transfer::Protocol`（串口用）分开定义；两者的默认值都不序列化。`UploadState` 没有实现 `Serialize`，要映射成 `upload-progress` / `upload-finished` 的 DTO。

## 不在本次范围内
- 下载：`sz`，以及设备主动发起 ZMODEM 时的自动检测。
- 断点续传。
- 上传文件夹。
- 没有 `rz` 时的 base64 等替代方案。
- TUI 端的支持。
