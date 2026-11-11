# Q. 拖放上传：前端 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户把本地文件拖到 SSH 终端窗格或串口标签页上就能上传：拖动经过时显示遮罩，上传中显示进度条和"取消"，ZMODEM/YMODEM/XMODEM 期间挡住键盘输入；主机和串口设备表单可以设置上传协议，终端和串口标签页可以临时改协议。后端（计划 P）已经完成，这份计划只改前端。

**Architecture:**
- `stores/uploads.ts`：按后端会话 id 记录每个上传（阶段、进度、速率），处理 `upload-progress` / `upload-finished`，提供开始上传、结束、进度文字等纯函数。
- `screens/dropTargets.ts`：Tauri 在页面之前接走系统的文件拖放，并带坐标报告；启动时挂一个监听，用 `document.elementFromPoint` 找到坐标下登记过的视图（终端窗格、串口终端区），更新悬停状态，松手时交给该视图。
- `screens/UploadStatus.svelte`：遮罩和进度条，终端窗格和串口视图共用。
- `TerminalPane.svelte`、`SerialView.svelte` 登记拖放目标、挂上 `UploadStatus`、上传期间丢掉键盘输入、提供协议下拉框；`HostEditor`、`SerialDeviceEditor` 加协议字段。

**Tech Stack:** Svelte 5（runes）、TypeScript、Tailwind、`@tauri-apps/api` 2.11（`getCurrentWebview().onDragDropEvent`）、Vitest、Playwright。不改 Rust，bindings 已经齐全（计划 P）。

**Spec:** `docs/superpowers/specs/2026-11-07-drag-drop-upload-design.md`；后端接口见 `docs/superpowers/plans/2026-11-11-p-upload-wiring.md` 的"给计划 Q 的接口"一节。

**命令约定（沙箱限制，务必遵守）：**
- 前台命令如果 60–100 秒没有输出会被沙箱杀掉。前端命令都用 `cd crates/omnyssh-gui/ui && nohup sh -c '…; echo EXIT=$?' > /tmp/q.log 2>&1 &` 放到后台，再反复 `tail -20 /tmp/q.log` 直到出现 `EXIT=`。
- Playwright 每次运行会先 `npm run build`，单个 spec 大约 30 s，全量大约 1 分钟。
- **补丁的用法：** 每个代码步骤给出一个补丁。用 Write 工具把它原样存到指定的 `/tmp/…` 文件（不要重排、不要改空格），然后在仓库根目录 `git apply --check <文件> && git apply <文件>`。`--check` 失败就停下来报告，不要手工"修"补丁。
- 这份计划不需要编译 Rust，不要跑 cargo。

---

## 实现中确定的设计决定

> **执行记录**：执行中各任务的代码审查又发现并修复了若干问题（Windows 以外的拖放坐标、不带文件的拖动、长粘贴混进上传流、分屏标题栏里的下拉框打不开、下拉框配色和焦点、取消后键盘不回终端、终端连上之前就接受拖放、关闭标签后的迟到错误、速率取样），设计决定已按最终行为更新。最终：vitest 479 个、Playwright 109 个（原有 97 个，`upload.spec.ts` 11 个，`terminal.spec.ts` 新增 1 个）。各任务里的补丁是最初版本，最终代码以仓库为准。

> 写这份计划时，整套改动已经在仓库的临时副本里完整实现并验证过：`npm run check`（0 错误 0 警告）、`npx vitest run`（470 个）、全量 Playwright（104 个，原有 97 个加新增 7 个）都通过。之后按下面 5 个任务在另一个副本里逐个重放、逐个验证，补丁就是从这串提交生成的，又在第三个副本上按顺序应用验证过：每个测试补丁之后相关测试失败（失败情况写在各任务里），每个实现补丁之后通过；最后连跑 3 遍全量 Playwright 都通过。

1. **拖放怎么接收**：Tauri 2 默认开启 `dragDropEnabled`，系统的文件拖放会被 Tauri 接走，页面自己的 HTML5 拖放事件不会触发。所以在 `+layout.svelte` 启动时挂一个 `getCurrentWebview().onDragDropEvent` 监听（全局只有一个）。事件里的坐标虽然标成 `PhysicalPosition`，Tauri 其实没有换算，是 wry 原样给的：Windows（WebView2，`ScreenToClient`）是物理像素，要除以 `devicePixelRatio`；macOS（`draggingLocation`，AppKit point）和 Linux（GTK 控件坐标）本身就是页面坐标，不能再除。（执行中修正：计划原本对所有平台都除以 `devicePixelRatio`，在 Retina 屏上文件会落到错误的窗格。执行时还加了：拖动的不是文件（macOS 会对文字、链接拖动发出不带路径的 enter）时不显示遮罩；悬停状态没变时不重写 store。）不在 Tauri 里（普通浏览器、没有 webview 元数据的测试页）时，监听什么都不做。
2. **命中判断**：视图把自己的根元素登记为拖放目标（`registerDropTarget`），监听用 `document.elementFromPoint` 找坐标下的元素，再往上找到登记过的根元素。隐藏的标签页是 `display: none`，自然点不中。分屏时每个窗格各自登记，拖到哪个窗格就传给哪个窗格的会话。
3. **上传状态**按后端会话 id 记在 `uploads` store 里：
   - 发出命令时进入 `starting`（SFTP 显示"Connecting for SFTP…"，可能停在密码提示上），收到 `waiting: true` 进入 `waiting`，收到进度进入 `sending`。
   - 收到 `upload-finished` 时清掉；会话结束（窗格销毁：用户关闭或 `terminal-exited`；串口断开；串口重连；关闭标签页）时调用 `endUpload` 清掉，因为这些情况可能没有 `upload-finished`（见计划 P 的接口说明）。
   - 命令返回错误：清掉，在状态栏显示 `Upload: <原因>`；错误是 `cancelled`（SFTP 连接建立期间取消）时不算错误，什么都不显示；会话在命令返回前已经结束（例如 SFTP 连接建立期间关掉了窗格）时，迟到的错误也不显示。
   - 结束时不弹提示：终端里的结果行由后端打印。
4. **界面文字用英文**（和整个界面一致），spec 里的中文只是意思：
   - 遮罩：`Drop to upload with ZMODEM to web-1`；YMODEM 多文件时加一行 `A bootloader such as U-Boot takes only the first file`；XMODEM 多文件时遮罩直接显示 `XMODEM sends one file at a time`。
   - 进度条：`Starting ZMODEM…` / `Connecting for SFTP…` / `Waiting for rz…`（ZMODEM）/ `Waiting for the receiver…`（Y/XMODEM）/ `fw.bin · 2/3 · 50% · 1.2 MB/s`，右边一个 `Cancel` 按钮。速率按同一个文件相邻两次进度平滑计算（权重 0.3），换文件时重算；ZMODEM 进度倒退时不更新速率。
5. **上传期间的键盘输入**：ZMODEM/YMODEM/XMODEM 进行时，窗格和串口的 `sendInput` 直接丢掉要发的字节（键盘、粘贴、快捷命令栏都走这里；长粘贴分块发送时每一块都再查一次）；SFTP 不影响输入。没有另外把输入框做成禁用状态。点 Cancel 不会把焦点从终端拿走，结束后可以直接接着打字。
6. **协议下拉框**（spec 的"临时覆盖"）：
   - 终端：放在悬停工具栏和分屏标题栏里，选项 ZMODEM / SFTP；默认取主机的 `uploadProtocol`（没有时 ZMODEM），改了只对这个窗格生效，不写回配置。
   - 串口：放在标签页顶部栏，选项 ZMODEM / YMODEM / XMODEM；从已保存设备打开时取设备的 `uploadProtocol`，临时打开的串口默认 ZMODEM。
   - 上传进行中下拉框锁定。按仓库规则（`select.test.ts`），下拉框一律用 `Select.svelte`，不能直接写 `<select>`；终端下拉框用 Svelte 5 的函数式绑定，显示"覆盖值，没有覆盖时用主机设置"。
7. **只接收（monitor）模式的串口标签页**不发数据，所以不登记拖放目标、也不显示协议下拉框；拖到上面没有任何反应。
8. **前端的预检查**（其余由后端检查，后端的拒绝原样显示）：XMODEM 多文件、这个会话已有上传、终端还没连上（状态不是 connected 时：`The terminal is still connecting`，连接失败时 `The terminal is not connected`）、串口没有打开（`COM3 is not open`）。这些情况在拖动经过时遮罩就直接显示拒绝原因。
9. **表单**：
   - 主机：`Drag-and-drop upload` 下拉框（`ZMODEM (rz)` / `SFTP`），选 SFTP 时出现 `Upload folder` 输入框（空表示 home）。保存时总是带上 `uploadProtocol`（后端把省略理解为"保持原值"，而且目录跟着协议走），`uploadDir` 只在 SFTP 时发送。
   - 串口设备：`Drag-and-drop upload` 下拉框（ZMODEM / YMODEM / XMODEM），保存时带上 `uploadProtocol`，修掉计划 P 留下的"保存会重置为 ZMODEM"。
10. **修了一个原有的测试竞态**（`e2e/terminal.spec.ts` 的 `holdConnect` 桩）：`__releaseConnect` 只给已经发起 `terminal_open` 的窗格发提示符，而窗格要先动态加载 xterm 才会发起；释放来得早，提示符就永远不发。新增的启动时拖放监听让这个用例在全量运行里从偶发变成经常失败。改成记住"已释放"，之后发起的窗格直接连上。
11. **没有在真实应用里验证拖放**：系统级的文件拖放没法在测试里驱动（Playwright 里用桩发 `tauri://drag-*` 事件代替）。Task 6 列了一份手动验证清单。

---

## 文件结构

| 文件 | 任务 | 职责 |
|---|---|---|
| `ui/src/lib/ipc/commands.ts` | 1 | `terminalUpload`、`serialUpload`、`uploadCancel` 封装 |
| `ui/src/lib/stores/uploads.ts`（新建）+ `uploads.test.ts` | 1 | 上传状态、事件处理、预检查、进度文字 |
| `ui/src/lib/screens/dropTargets.ts`（新建）+ `dropTargets.test.ts` | 2 | 拖放目标登记、命中判断、全局监听 |
| `ui/src/lib/ipc/subscribe.ts` + `subscribe.test.ts` | 2 | 订阅 `upload-progress` / `upload-finished` |
| `ui/src/routes/+layout.svelte` | 2 | 启动拖放监听 |
| `ui/src/lib/screens/UploadStatus.svelte`（新建） | 3 | 遮罩和进度条 |
| `ui/src/lib/screens/TerminalPane.svelte` | 3 | 终端窗格接入 |
| `ui/e2e/upload.spec.ts`（新建） | 3、4、5 | 拖放上传的 e2e 用例（每个任务加几个） |
| `ui/e2e/terminal.spec.ts` | 3 | 修 `holdConnect` 桩的竞态 |
| `ui/src/lib/screens/SerialView.svelte`、`stores/sessions.ts`、`screens/Dashboard.svelte` | 4 | 串口标签页接入，设备协议带进标签页 |
| `ui/src/lib/screens/hostForm.ts`、`HostEditor.svelte`、`serialForm.ts`、`SerialDeviceEditor.svelte`（及测试） | 5 | 表单字段 |

（`ui/` 指 `crates/omnyssh-gui/ui/`。）

---

### Task 1: 上传状态与命令封装

**Files:** `ui/src/lib/ipc/commands.ts`；Create `ui/src/lib/stores/uploads.ts`、`ui/src/lib/stores/uploads.test.ts`

`startUpload` 接收一个发送函数（`terminalUpload` 或 `serialUpload`），这样测试不需要 mock IPC。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q1-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q1-test.patch && git apply /tmp/q1-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/stores/uploads.test.ts b/crates/omnyssh-gui/ui/src/lib/stores/uploads.test.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/stores/uploads.test.ts
@@ -0,0 +1,160 @@
+import { describe, it, expect, beforeEach } from 'vitest';
+import { get } from 'svelte/store';
+import {
+  applyUploadFinished,
+  applyUploadProgress,
+  blocksInput,
+  dropError,
+  dropNote,
+  endUpload,
+  formatSize,
+  progressLine,
+  startUpload,
+  uploads,
+  type UploadProtocol
+} from './uploads';
+import { lastError } from './notifications';
+
+const progress = (over: Partial<Parameters<typeof applyUploadProgress>[0]> = {}) => ({
+  sessionId: 7,
+  waiting: false,
+  fileIndex: 0,
+  fileCount: 1,
+  fileName: 'a.bin',
+  sent: 0,
+  total: 1000,
+  ...over
+});
+
+/** A send that records what it was asked and resolves. */
+function recorder() {
+  const calls: [number, UploadProtocol, string[]][] = [];
+  const send = (id: number, p: UploadProtocol, paths: string[]) => {
+    calls.push([id, p, paths]);
+    return Promise.resolve();
+  };
+  return { calls, send };
+}
+
+beforeEach(() => {
+  uploads.set(new Map());
+  lastError.set(null);
+});
+
+describe('startUpload', () => {
+  it('starts in the starting phase and sends the drop', async () => {
+    const { calls, send } = recorder();
+    await startUpload(7, 'zmodem', ['/a.bin', '/b.bin'], send);
+    expect(calls).toEqual([[7, 'zmodem', ['/a.bin', '/b.bin']]]);
+    const view = get(uploads).get(7)!;
+    expect(view.phase).toBe('starting');
+    expect(view.fileCount).toBe(2);
+  });
+
+  it('refuses several files for XMODEM before sending anything', async () => {
+    const { calls, send } = recorder();
+    await startUpload(7, 'xmodem', ['/a', '/b'], send);
+    expect(calls).toEqual([]);
+    expect(get(lastError)).toBe('XMODEM sends one file at a time');
+    expect(get(uploads).has(7)).toBe(false);
+  });
+
+  it('refuses a second upload in the same session', async () => {
+    const { calls, send } = recorder();
+    await startUpload(7, 'zmodem', ['/a'], send);
+    await startUpload(7, 'zmodem', ['/b'], send);
+    expect(calls).toHaveLength(1);
+    expect(get(lastError)).toBe('An upload is already running here');
+  });
+
+  it('drops the upload and reports a refusal from the backend', async () => {
+    await startUpload(7, 'zmodem', ['/dir'], () =>
+      Promise.reject(new Error('folders cannot be uploaded: dir'))
+    );
+    expect(get(uploads).has(7)).toBe(false);
+    expect(get(lastError)).toBe('Upload: folders cannot be uploaded: dir');
+  });
+
+  it('treats a cancel during the SFTP login as a cancel, not an error', async () => {
+    await startUpload(7, 'sftp', ['/a'], () => Promise.reject(new Error('cancelled')));
+    expect(get(uploads).has(7)).toBe(false);
+    expect(get(lastError)).toBeNull();
+  });
+});
+
+describe('progress', () => {
+  it('moves from waiting to sending and computes a smoothed rate', async () => {
+    await startUpload(7, 'zmodem', ['/a.bin'], recorder().send);
+    applyUploadProgress(progress({ waiting: true, fileName: '', total: 0 }), 0);
+    expect(get(uploads).get(7)!.phase).toBe('waiting');
+    applyUploadProgress(progress({ sent: 0 }), 1000);
+    expect(get(uploads).get(7)!.rate).toBe(0);
+    applyUploadProgress(progress({ sent: 100 }), 2000);
+    expect(get(uploads).get(7)!.rate).toBe(100);
+    applyUploadProgress(progress({ sent: 400 }), 3000);
+    // 100 + 0.3 * (300 - 100)
+    expect(get(uploads).get(7)!.rate).toBeCloseTo(160);
+  });
+
+  it('starts the rate over on the next file, and ignores a resend going back', async () => {
+    await startUpload(7, 'zmodem', ['/a', '/b'], recorder().send);
+    applyUploadProgress(progress({ sent: 0, fileCount: 2 }), 0);
+    applyUploadProgress(progress({ sent: 500, fileCount: 2 }), 1000);
+    applyUploadProgress(progress({ sent: 300, fileCount: 2 }), 1500);
+    expect(get(uploads).get(7)!.rate).toBe(500);
+    applyUploadProgress(progress({ fileIndex: 1, fileName: 'b', sent: 10, fileCount: 2 }), 2000);
+    expect(get(uploads).get(7)!.rate).toBe(0);
+  });
+
+  it('ignores progress for a session it does not know', () => {
+    applyUploadProgress(progress({ sessionId: 99 }));
+    expect(get(uploads).size).toBe(0);
+  });
+
+  it('ends on upload-finished and on endUpload', async () => {
+    await startUpload(7, 'zmodem', ['/a'], recorder().send);
+    applyUploadFinished({ sessionId: 7, ok: true, message: 'uploaded a (1 B)' });
+    expect(get(uploads).has(7)).toBe(false);
+    await startUpload(8, 'sftp', ['/a'], recorder().send);
+    endUpload(8);
+    endUpload(undefined);
+    expect(get(uploads).size).toBe(0);
+  });
+});
+
+describe('display', () => {
+  it('blocks input for the protocols that own the stream', () => {
+    const base = { phase: 'sending' as const, fileIndex: 0, fileCount: 1, fileName: '', sent: 0, total: 0, rate: 0, sampleAt: 0, sampleSent: 0 };
+    expect(blocksInput(undefined)).toBe(false);
+    expect(blocksInput({ ...base, protocol: 'ymodem' })).toBe(true);
+    expect(blocksInput({ ...base, protocol: 'sftp' })).toBe(false);
+  });
+
+  it('writes the progress line', () => {
+    const base = { protocol: 'zmodem' as const, fileIndex: 1, fileCount: 3, fileName: 'b.bin', sent: 512, total: 1024, rate: 1536, sampleAt: 0, sampleSent: 0 };
+    expect(progressLine({ ...base, phase: 'sending' })).toEqual({
+      text: 'b.bin · 2/3 · 50% · 1.5 KB/s',
+      fraction: 0.5
+    });
+    expect(progressLine({ ...base, phase: 'waiting' }).text).toBe('Waiting for rz…');
+    expect(progressLine({ ...base, protocol: 'ymodem', phase: 'waiting' }).text).toBe('Waiting for the receiver…');
+    expect(progressLine({ ...base, protocol: 'sftp', phase: 'starting' })).toEqual({
+      text: 'Connecting for SFTP…',
+      fraction: null
+    });
+    expect(progressLine({ ...base, phase: 'sending', fileCount: 1, total: 0, rate: 0 }).text).toBe('b.bin · 100%');
+  });
+
+  it('notes and refusals for the drop overlay', () => {
+    expect(dropError('xmodem', 1)).toBeNull();
+    expect(dropError('xmodem', 2)).not.toBeNull();
+    expect(dropNote('ymodem', 2)).toContain('U-Boot');
+    expect(dropNote('zmodem', 2)).toBeNull();
+  });
+
+  it('formats sizes like the backend', () => {
+    expect(formatSize(1023)).toBe('1023 B');
+    expect(formatSize(1024)).toBe('1.0 KB');
+    expect(formatSize(1_048_575)).toBe('1.0 MB');
+  });
+});
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/stores/uploads.test.ts 2>&1 | grep -E "Tests |Error" | head -5; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: vitest 报 `Failed to load url ./uploads … Does the file exist?`，没有测试运行。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q1-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q1-impl.patch && git apply /tmp/q1-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts b/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/commands.ts
@@ -7,6 +7,7 @@
   FileEntryDto,
   HostDto,
   HostInputDto,
+  HostUploadProtocolDto,
   HostKeyDecisionDto,
   LogModeDto,
   LogStatusDto,
@@ -16,6 +17,7 @@
   SerialDeviceDto,
   SerialExitDto,
   SerialPortDto,
+  SerialUploadProtocolDto,
   SnippetDto,
   TerminalBytes,
   TraySupportDto,
@@ -394,3 +396,30 @@
   const res = await commands.revealPath(path);
   if (res.status === 'error') throw new Error(res.error.message);
 }
+
+/** Upload `paths` into terminal `sessionId` (ZMODEM to `rz`, or SFTP). Resolves once it
+ *  started; `upload-progress` and `upload-finished` follow. */
+export async function terminalUpload(
+  sessionId: number,
+  protocol: HostUploadProtocolDto,
+  paths: string[]
+): Promise<void> {
+  const res = await commands.terminalUpload(sessionId, protocol, paths);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** Upload `paths` to serial port `sessionId`. */
+export async function serialUpload(
+  sessionId: number,
+  protocol: SerialUploadProtocolDto,
+  paths: string[]
+): Promise<void> {
+  const res = await commands.serialUpload(sessionId, protocol, paths);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
+
+/** Cancel session `sessionId`'s upload, if any. */
+export async function uploadCancel(sessionId: number): Promise<void> {
+  const res = await commands.uploadCancel(sessionId);
+  if (res.status === 'error') throw new Error(res.error.message);
+}
diff --git a/crates/omnyssh-gui/ui/src/lib/stores/uploads.ts b/crates/omnyssh-gui/ui/src/lib/stores/uploads.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/stores/uploads.ts
@@ -0,0 +1,183 @@
+// Drag-and-drop uploads in terminal and serial tabs (plan Q), keyed by the backend
+// session id. A tab starts one with `startUpload`; the backend's `upload-progress`
+// and `upload-finished` move it along; the tab's end (`terminal-exited`, a serial
+// exit, closing it) drops it with `endUpload`, because an upload whose session ends
+// first gets no `upload-finished`. The terminal shows the result line itself (the
+// backend prints it), so nothing here reports success.
+import { writable, get } from 'svelte/store';
+import type { UploadFinished, UploadProgress } from '$lib/bindings';
+import { lastError } from './notifications';
+
+export type UploadProtocol = 'zmodem' | 'ymodem' | 'xmodem' | 'sftp';
+
+export interface UploadView {
+  protocol: UploadProtocol;
+  /** `starting` until the backend reports: the command is running (an SFTP
+   *  connection may be logging in); `waiting` for the receiver (`rz`, `loady`). */
+  phase: 'starting' | 'waiting' | 'sending';
+  fileIndex: number;
+  fileCount: number;
+  fileName: string;
+  sent: number;
+  total: number;
+  /** Bytes per second, smoothed; 0 until there are two samples of one file. */
+  rate: number;
+  /** The last sample the rate was taken from (ms, `sent`). */
+  sampleAt: number;
+  sampleSent: number;
+}
+
+export const uploads = writable<Map<number, UploadView>>(new Map());
+
+const NAMES: Record<UploadProtocol, string> = {
+  zmodem: 'ZMODEM',
+  ymodem: 'YMODEM',
+  xmodem: 'XMODEM',
+  sftp: 'SFTP'
+};
+
+export function protocolName(p: UploadProtocol): string {
+  return NAMES[p];
+}
+
+/** ZMODEM, YMODEM and XMODEM own the byte stream while they run: keystrokes would
+ *  land in the middle of it. SFTP runs beside the shell. */
+export function blocksInput(view: UploadView | undefined): boolean {
+  return view != null && view.protocol !== 'sftp';
+}
+
+/** Whether a drop of `count` files can go by `protocol`, checked before anything is
+ *  sent. The backend checks the rest (folders, sizes, names). */
+export function dropError(protocol: UploadProtocol, count: number): string | null {
+  return protocol === 'xmodem' && count > 1 ? 'XMODEM sends one file at a time' : null;
+}
+
+/** A note for the drop overlay: U-Boot's `loady` takes only the first file of a batch. */
+export function dropNote(protocol: UploadProtocol, count: number): string | null {
+  return protocol === 'ymodem' && count > 1
+    ? 'A bootloader such as U-Boot takes only the first file'
+    : null;
+}
+
+function set(id: number, view: UploadView | null): void {
+  uploads.update((m) => {
+    const next = new Map(m);
+    if (view) next.set(id, view);
+    else next.delete(id);
+    return next;
+  });
+}
+
+/** Starts an upload on backend session `id` through `send` (`terminalUpload` or
+ *  `serialUpload`). Refusals go to the status bar; a cancel during an SFTP login
+ *  comes back as the error `cancelled` and is not one. */
+export async function startUpload(
+  id: number,
+  protocol: UploadProtocol,
+  paths: string[],
+  send: (id: number, protocol: UploadProtocol, paths: string[]) => Promise<void>
+): Promise<void> {
+  const refused = dropError(protocol, paths.length);
+  if (refused) {
+    lastError.set(refused);
+    return;
+  }
+  if (get(uploads).has(id)) {
+    lastError.set('An upload is already running here');
+    return;
+  }
+  set(id, {
+    protocol,
+    phase: 'starting',
+    fileIndex: 0,
+    fileCount: paths.length,
+    fileName: '',
+    sent: 0,
+    total: 0,
+    rate: 0,
+    sampleAt: 0,
+    sampleSent: 0
+  });
+  try {
+    await send(id, protocol, paths);
+  } catch (e) {
+    endUpload(id);
+    const message = e instanceof Error ? e.message : String(e);
+    if (message !== 'cancelled') lastError.set(`Upload: ${message}`);
+  }
+}
+
+/** How much a new rate sample counts against the running rate. */
+const RATE_WEIGHT = 0.3;
+
+export function applyUploadProgress(p: UploadProgress, now: number = Date.now()): void {
+  const view = get(uploads).get(p.sessionId);
+  // Unknown: the tab has gone, or the upload ended already.
+  if (!view) return;
+  if (p.waiting) {
+    set(p.sessionId, { ...view, phase: 'waiting' });
+    return;
+  }
+  const sameFile = view.phase === 'sending' && view.fileIndex === p.fileIndex;
+  let rate = sameFile ? view.rate : 0;
+  if (sameFile && now > view.sampleAt && p.sent > view.sampleSent) {
+    const sample = ((p.sent - view.sampleSent) * 1000) / (now - view.sampleAt);
+    rate = rate === 0 ? sample : rate + RATE_WEIGHT * (sample - rate);
+  }
+  set(p.sessionId, {
+    ...view,
+    phase: 'sending',
+    fileIndex: p.fileIndex,
+    fileCount: p.fileCount,
+    fileName: p.fileName,
+    sent: p.sent,
+    total: p.total,
+    rate,
+    sampleAt: now,
+    sampleSent: p.sent
+  });
+}
+
+/** The upload ended: the terminal already shows its result line. */
+export function applyUploadFinished(f: UploadFinished): void {
+  endUpload(f.sessionId);
+}
+
+/** Forget session `id`'s upload: it finished, or its session ended. */
+export function endUpload(id: number | undefined): void {
+  if (id != null && get(uploads).has(id)) set(id, null);
+}
+
+/** `512 B`, `1.2 KB`, `3.4 MB` (powers of 1024, like the backend's result line). */
+export function formatSize(bytes: number): string {
+  const units = ['KB', 'MB', 'GB', 'TB'];
+  if (bytes < 1024) return `${Math.round(bytes)} B`;
+  let value = bytes / 1024;
+  let unit = 0;
+  while (value >= 1023.95 && unit < units.length - 1) {
+    value /= 1024;
+    unit++;
+  }
+  return `${value.toFixed(1)} ${units[unit]}`;
+}
+
+/** What the progress bar shows: its text, and how full it is (0..1, or null while
+ *  there is nothing to measure). */
+export function progressLine(view: UploadView): { text: string; fraction: number | null } {
+  const name = protocolName(view.protocol);
+  if (view.phase === 'starting') {
+    return { text: view.protocol === 'sftp' ? 'Connecting for SFTP…' : `Starting ${name}…`, fraction: null };
+  }
+  if (view.phase === 'waiting') {
+    return {
+      text: view.protocol === 'zmodem' ? 'Waiting for rz…' : 'Waiting for the receiver…',
+      fraction: null
+    };
+  }
+  const fraction = view.total > 0 ? Math.min(1, view.sent / view.total) : 1;
+  const parts = [view.fileName];
+  if (view.fileCount > 1) parts.push(`${view.fileIndex + 1}/${view.fileCount}`);
+  parts.push(`${Math.floor(fraction * 100)}%`);
+  if (view.rate > 0) parts.push(`${formatSize(view.rate)}/s`);
+  return { text: parts.join(' · '), fraction };
+}
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/stores/uploads.test.ts 2>&1 | grep -E "Tests |×"; npm run check 2>&1 | tail -1; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: 13 个测试通过；svelte-check `0 ERRORS 0 WARNINGS`。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/ipc/commands.ts crates/omnyssh-gui/ui/src/lib/stores/uploads.ts crates/omnyssh-gui/ui/src/lib/stores/uploads.test.ts
git commit -m "feat(ui): upload state, its events and the upload commands"
```

---

### Task 2: 拖放监听与事件订阅

**Files:** Create `ui/src/lib/screens/dropTargets.ts`、`dropTargets.test.ts`；Modify `ui/src/lib/ipc/subscribe.ts`、`subscribe.test.ts`、`ui/src/routes/+layout.svelte`

`createDragTracker` 把命中判断函数作为参数，测试里用 jsdom 元素代替 `document.elementFromPoint`。`subscribe.test.ts` 的 `vi.mock('$lib/bindings')` 要加上两个新事件的 channel，否则 `startEventBridge` 会因为事件不存在而失败（补丁里已包含）。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q2-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q2-test.patch && git apply /tmp/q2-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts
@@ -39,6 +39,8 @@
       passwordRequired: channel('passwordRequired'),
       hostKeyChanged: channel('hostKeyChanged'),
       logStopped: channel('logStopped'),
+      uploadProgress: channel('uploadProgress'),
+      uploadFinished: channel('uploadFinished'),
       error: channel('error')
     }
   };
@@ -57,6 +59,7 @@
 import { hostKeyPrompt, hostKeyQueue } from '$lib/stores/hostKey';
 import { sessionLogs, logStarted } from '$lib/stores/sessionLogs';
 import { statusNotice } from '$lib/stores/notifications';
+import { startUpload, uploads } from '$lib/stores/uploads';
 import { startEventBridge } from './subscribe';
 
 describe('startEventBridge', () => {
@@ -133,6 +136,17 @@
     statusNotice.set(null);
   });
 
+  it('upload events move the upload along and end it', async () => {
+    await startEventBridge();
+    await startUpload(42, 'zmodem', ['/a.bin'], () => Promise.resolve());
+    listeners.uploadProgress({
+      payload: { sessionId: 42, waiting: true, fileIndex: 0, fileCount: 0, fileName: '', sent: 0, total: 0 }
+    });
+    expect(get(uploads).get(42)?.phase).toBe('waiting');
+    listeners.uploadFinished({ payload: { sessionId: 42, ok: true, message: 'uploaded a.bin (1 B)' } });
+    expect(get(uploads).has(42)).toBe(false);
+  });
+
   it('terminal-exited closes the tab whose backend id matches', async () => {
     await startEventBridge();
     const tab = sessions.spawn('terminal', 'web-1');
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.test.ts b/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.test.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.test.ts
@@ -0,0 +1,74 @@
+// @vitest-environment jsdom
+import { describe, it, expect, beforeEach } from 'vitest';
+import { get } from 'svelte/store';
+import { createDragTracker, dropHover, registerDropTarget, targetFor, type DropTarget } from './dropTargets';
+import type { UploadProtocol } from '$lib/stores/uploads';
+
+function target(protocol: UploadProtocol, dropped: string[][]): DropTarget {
+  return { label: () => 'web-1', protocol: () => protocol, drop: (paths) => dropped.push(paths) };
+}
+
+describe('drop targets', () => {
+  let root: HTMLElement;
+  let inner: HTMLElement;
+  let outside: HTMLElement;
+
+  beforeEach(() => {
+    document.body.innerHTML = '<div id="root"><div><span id="inner"></span></div></div><p id="out"></p>';
+    root = document.getElementById('root')!;
+    inner = document.getElementById('inner')!;
+    outside = document.getElementById('out')!;
+    dropHover.set(null);
+  });
+
+  it('finds the registered view an element is inside', () => {
+    const off = registerDropTarget(root, target('zmodem', []));
+    expect(targetFor(inner)?.el).toBe(root);
+    expect(targetFor(outside)).toBeNull();
+    expect(targetFor(null)).toBeNull();
+    off();
+    expect(targetFor(inner)).toBeNull();
+  });
+
+  it('shows the overlay while dragging over a view and drops there', () => {
+    const dropped: string[][] = [];
+    const off = registerDropTarget(root, target('sftp', dropped));
+    let under: Element | null = inner;
+    const track = createDragTracker(() => under);
+
+    track({ type: 'enter', paths: ['/a', '/b'], x: 1, y: 1 });
+    expect(get(dropHover)).toEqual({ el: root, label: 'web-1', protocol: 'sftp', count: 2 });
+    under = outside;
+    track({ type: 'over', x: 2, y: 2 });
+    expect(get(dropHover)).toBeNull();
+    under = inner;
+    track({ type: 'over', x: 1, y: 1 });
+    expect(get(dropHover)?.el).toBe(root);
+    track({ type: 'drop', paths: ['/a', '/b'], x: 1, y: 1 });
+    expect(dropped).toEqual([['/a', '/b']]);
+    expect(get(dropHover)).toBeNull();
+    off();
+  });
+
+  it('ignores a drop outside every view, and a cancelled drag', () => {
+    const dropped: string[][] = [];
+    const off = registerDropTarget(root, target('zmodem', dropped));
+    const track = createDragTracker(() => outside);
+    track({ type: 'enter', paths: ['/a'], x: 0, y: 0 });
+    track({ type: 'drop', paths: ['/a'], x: 0, y: 0 });
+    expect(dropped).toEqual([]);
+    const over = createDragTracker(() => inner);
+    over({ type: 'enter', paths: ['/a'], x: 0, y: 0 });
+    over({ type: 'leave' });
+    expect(get(dropHover)).toBeNull();
+    expect(dropped).toEqual([]);
+    off();
+  });
+
+  it('an old registration cannot remove a newer one for the same element', () => {
+    const first = registerDropTarget(root, target('zmodem', []));
+    registerDropTarget(root, target('xmodem', []));
+    first();
+    expect(targetFor(root)?.target.protocol()).toBe('xmodem');
+  });
+});
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/screens/dropTargets.test.ts src/lib/ipc/subscribe.test.ts 2>&1 | grep -E "Tests |×|Error" | head -6; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `dropTargets.test.ts` 报 `Failed to resolve import "./dropTargets"`；`subscribe.test.ts` 里新用例 `upload events move the upload along and end it` 失败（`listeners.uploadProgress is not a function`），其余 6 个通过。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q2-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q2-impl.patch && git apply /tmp/q2-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
--- a/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
+++ b/crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts
@@ -30,6 +30,7 @@
   applyHostKeyChanged
 } from './router';
 import { applyLogStopped } from '$lib/stores/sessionLogs';
+import { applyUploadFinished, applyUploadProgress } from '$lib/stores/uploads';
 
 export async function startEventBridge(): Promise<() => void> {
   const offs: UnlistenFn[] = [];
@@ -57,6 +58,8 @@
     offs.push(await events.passwordRequired.listen((e) => applyPasswordRequired(e.payload)));
     offs.push(await events.hostKeyChanged.listen((e) => applyHostKeyChanged(e.payload)));
     offs.push(await events.logStopped.listen((e) => applyLogStopped(e.payload)));
+    offs.push(await events.uploadProgress.listen((e) => applyUploadProgress(e.payload)));
+    offs.push(await events.uploadFinished.listen((e) => applyUploadFinished(e.payload)));
     offs.push(await events.error.listen((e) => applyError(e.payload.message)));
   } catch (err) {
     offs.forEach((off) => off());
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.ts b/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/dropTargets.ts
@@ -0,0 +1,105 @@
+// Where a file dropped on the window lands (plan Q). Tauri takes file drops from the OS
+// before the page sees them and reports them with a position, so the page's own drag
+// events never fire; one listener, started at boot, finds the terminal or serial view
+// under the pointer. Views register their root element; a hidden tab is `display:
+// none`, so only what is on screen can be hit.
+import { writable } from 'svelte/store';
+import type { UploadProtocol } from '$lib/stores/uploads';
+
+export interface DropTarget {
+  /** The session name the overlay names, e.g. `web-1` or `COM3 · 115200`. */
+  label: () => string;
+  /** The protocol a drop would use now (the tab's override, else its setting). */
+  protocol: () => UploadProtocol;
+  /** Files were dropped here. */
+  drop: (paths: string[]) => void;
+}
+
+/** What is being dragged over, for the overlay of the view whose root is `el`. */
+export interface DropHover {
+  el: Element;
+  label: string;
+  protocol: UploadProtocol;
+  count: number;
+}
+
+const targets = new Map<Element, DropTarget>();
+
+export const dropHover = writable<DropHover | null>(null);
+
+export function registerDropTarget(el: Element, target: DropTarget): () => void {
+  targets.set(el, target);
+  return () => {
+    if (targets.get(el) === target) targets.delete(el);
+  };
+}
+
+/** The registered view `el` is inside, if any. */
+export function targetFor(el: Element | null): { el: Element; target: DropTarget } | null {
+  for (let node = el; node; node = node.parentElement) {
+    const target = targets.get(node);
+    if (target) return { el: node, target };
+  }
+  return null;
+}
+
+/** What the OS reports, in the page's terms: Tauri gives physical pixels. */
+export type DragEvent =
+  | { type: 'enter'; paths: string[]; x: number; y: number }
+  | { type: 'over'; x: number; y: number }
+  | { type: 'drop'; paths: string[]; x: number; y: number }
+  | { type: 'leave' };
+
+/** Follows one drag: `hit` finds the element at a point (`document.elementFromPoint`
+ *  in the app). Updates `dropHover` and hands a drop to its target. Pure apart from
+ *  the store, so it is testable without a window. */
+export function createDragTracker(hit: (x: number, y: number) => Element | null) {
+  let count = 0;
+  function hover(x: number, y: number): { el: Element; target: DropTarget } | null {
+    const found = targetFor(hit(x, y));
+    dropHover.set(
+      found
+        ? { el: found.el, label: found.target.label(), protocol: found.target.protocol(), count }
+        : null
+    );
+    return found;
+  }
+  return (e: DragEvent): void => {
+    switch (e.type) {
+      case 'enter':
+        count = e.paths.length;
+        hover(e.x, e.y);
+        break;
+      case 'over':
+        hover(e.x, e.y);
+        break;
+      case 'drop': {
+        dropHover.set(null);
+        const found = targetFor(hit(e.x, e.y));
+        if (found && e.paths.length > 0) found.target.drop(e.paths);
+        break;
+      }
+      case 'leave':
+        dropHover.set(null);
+        break;
+    }
+  };
+}
+
+/** Starts following OS file drags over the window. Outside Tauri (a plain browser,
+ *  a test page without webview metadata) it does nothing. */
+export async function startDropListener(): Promise<() => void> {
+  try {
+    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
+    const track = createDragTracker((x, y) => document.elementFromPoint(x, y));
+    const scale = (v: number) => v / (window.devicePixelRatio || 1);
+    return await getCurrentWebview().onDragDropEvent((event) => {
+      const p = event.payload;
+      if (p.type === 'leave') track({ type: 'leave' });
+      else if (p.type === 'over') track({ type: 'over', x: scale(p.position.x), y: scale(p.position.y) });
+      else track({ type: p.type, paths: p.paths, x: scale(p.position.x), y: scale(p.position.y) });
+    });
+  } catch {
+    return () => {};
+  }
+}
diff --git a/crates/omnyssh-gui/ui/src/routes/+layout.svelte b/crates/omnyssh-gui/ui/src/routes/+layout.svelte
--- a/crates/omnyssh-gui/ui/src/routes/+layout.svelte
+++ b/crates/omnyssh-gui/ui/src/routes/+layout.svelte
@@ -2,6 +2,7 @@
   import '../app.css';
   import { onMount } from 'svelte';
   import { startEventBridge } from '$lib/ipc/subscribe';
+  import { startDropListener } from '$lib/screens/dropTargets';
   import { reloadHosts, refreshMetrics, setTrayBehavior } from '$lib/ipc/commands';
   import { theme } from '$lib/stores/theme';
   import { sidebarCollapsed } from '$lib/stores/ui';
@@ -74,9 +75,16 @@
         reloadHosts().catch((err) => lastError.set(err instanceof Error ? err.message : String(err)));
       })
       .catch(() => {});
+    // Files dragged onto a terminal or serial view upload there (plan Q).
+    let stopDrop: (() => void) | undefined;
+    void startDropListener().then((off) => {
+      if (disposed) off();
+      else stopDrop = off;
+    });
     return () => {
       disposed = true;
       stop?.();
+      stopDrop?.();
       stopRefresh();
       stopTray();
       stopHotkey();
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run 2>&1 | grep -E "Tests |×"; npm run check 2>&1 | tail -1; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: 全部 vitest 通过（新增 `dropTargets` 4 个、`subscribe` 1 个）；svelte-check 无错误。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/screens/dropTargets.ts crates/omnyssh-gui/ui/src/lib/screens/dropTargets.test.ts crates/omnyssh-gui/ui/src/lib/ipc/subscribe.ts crates/omnyssh-gui/ui/src/lib/ipc/subscribe.test.ts crates/omnyssh-gui/ui/src/routes/+layout.svelte
git commit -m "feat(ui): find the view a file is dragged onto"
```

---

### Task 3: 终端窗格

**Files:** Create `ui/src/lib/screens/UploadStatus.svelte`、`ui/e2e/upload.spec.ts`；Modify `ui/src/lib/screens/TerminalPane.svelte`、`ui/e2e/terminal.spec.ts`

e2e 的桩用 `__drag(type, x, y, paths)` 发出 Tauri 的 `tauri://drag-enter/over/drop/leave` 事件，并提供 `__TAURI_INTERNALS__.metadata`（`getCurrentWebview()` 需要）。这一步的 3 个用例覆盖：拖动遮罩、ZMODEM 上传、上传期间键盘输入被丢掉、取消、结束；主机设置和窗格覆盖决定协议、SFTP 不挡输入；后端拒绝时状态栏显示原因、拖到终端以外没有反应。同时修 `terminal.spec.ts` 里 `holdConnect` 桩的竞态（设计决定 10）。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q3-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q3-test.patch && git apply /tmp/q3-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/terminal.spec.ts b/crates/omnyssh-gui/ui/e2e/terminal.spec.ts
--- a/crates/omnyssh-gui/ui/e2e/terminal.spec.ts
+++ b/crates/omnyssh-gui/ui/e2e/terminal.spec.ts
@@ -30,7 +30,11 @@
       // Held back with `holdConnect`, so a test can keep a pane "connecting" until it
       // calls `__releaseConnect`.
       const pendingPrompts: number[] = [];
+      // A release can come before the pane has even asked to open (it loads xterm
+      // first): remember it, so that pane connects as soon as it does.
+      let released = false;
       (win as { __releaseConnect?: () => void }).__releaseConnect = () => {
+        released = true;
         for (const chId of pendingPrompts.splice(0)) sendToChannel(chId, 'omnyssh-ready> ');
       };
 
@@ -76,7 +80,7 @@
               // A shell prompt proves the streamed output renders + flips status to
               // connected; held back instead when a test wants the pane to stay
               // "connecting" until it releases it.
-              if (holdConnect) pendingPrompts.push(chId);
+              if (holdConnect && !released) pendingPrompts.push(chId);
               else setTimeout(() => sendToChannel(chId, 'omnyssh-ready> '), 0);
               return Promise.resolve(sid);
             }
diff --git a/crates/omnyssh-gui/ui/e2e/upload.spec.ts b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
@@ -0,0 +1,254 @@
+import { expect, test, type Locator, type Page } from '@playwright/test';
+
+// Drag-and-drop upload (plan Q). Tauri is absent, so the stub stands in for it: OS file
+// drags arrive as the `tauri://drag-*` events Tauri emits (fired with `__drag`), the
+// upload commands are recorded in `__calls`, and the backend's `upload-progress` /
+// `upload-finished` are fired with `__fire`. `__refuse` makes the next upload command
+// fail with that message.
+const HOSTS = [
+  { name: 'web-1', hostname: '10.0.0.5', user: 'deploy', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false },
+  { name: 'nas', hostname: '10.0.0.9', user: 'me', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false, uploadProtocol: 'sftp', uploadDir: '~/drop' }
+];
+const DEVICES = [
+  {
+    name: 'board',
+    config: { port: 'COM3', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' },
+    enter: 'cr',
+    notes: null,
+    uploadProtocol: 'ymodem'
+  }
+];
+
+type Call = { cmd: string; args: Record<string, unknown> };
+
+async function boot(page: Page): Promise<void> {
+  await page.addInitScript(
+    ({ hosts, devices }) => {
+      let cbid = 0;
+      const win = window as unknown as Record<string, unknown>;
+      const listeners: Record<string, number[]> = {};
+      const chIndex: Record<number, number> = {};
+      let nextSession = 0;
+      const calls: Call[] = [];
+      win.__calls = calls;
+      win.__refuse = null;
+
+      function send(chId: number, text: string): void {
+        const cb = win[`__cb${chId}`] as ((m: unknown) => void) | undefined;
+        const index = chIndex[chId] ?? 0;
+        chIndex[chId] = index + 1;
+        cb?.({ message: new TextEncoder().encode(text).buffer, index });
+      }
+      function fire(event: string, payload: unknown): void {
+        for (const id of listeners[event] ?? []) {
+          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
+          cb?.({ event, id, payload });
+        }
+      }
+      win.__fire = fire;
+      // Physical pixels, as Tauri reports them.
+      win.__drag = (type: string, x: number, y: number, paths: string[]) => {
+        const position = { x: x * devicePixelRatio, y: y * devicePixelRatio };
+        if (type === 'leave') fire('tauri://drag-leave', null);
+        else if (type === 'over') fire('tauri://drag-over', { position });
+        else fire(`tauri://drag-${type}`, { paths, position });
+      };
+
+      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
+        metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
+        invoke: (cmd: string, args: Record<string, unknown>) => {
+          switch (cmd) {
+            case 'list_hosts':
+              return Promise.resolve(hosts);
+            case 'list_serial_devices':
+              return Promise.resolve(devices);
+            case 'list_quick_commands':
+              return Promise.resolve([]);
+            case 'set_tray_behavior':
+              return Promise.resolve({ available: true, minimize: true });
+            case 'terminal_open': {
+              const chId = (args.onOutput as { id: number }).id;
+              const sid = ++nextSession;
+              setTimeout(() => send(chId, 'omnyssh-ready> '), 0);
+              return Promise.resolve(sid);
+            }
+            case 'serial_list_ports':
+              return Promise.resolve([{ name: 'COM3', description: null }]);
+            case 'serial_open':
+              return Promise.resolve(++nextSession);
+            case 'terminal_upload':
+            case 'serial_upload': {
+              calls.push({ cmd, args });
+              const refuse = win.__refuse as string | null;
+              win.__refuse = null;
+              return refuse ? Promise.reject({ message: refuse }) : Promise.resolve(null);
+            }
+            case 'upload_cancel':
+            case 'terminal_write':
+            case 'serial_write':
+            case 'save_host':
+            case 'save_serial_device':
+              calls.push({ cmd, args });
+              return Promise.resolve(null);
+            case 'plugin:event|listen': {
+              const { event, handler } = args as { event: string; handler: number };
+              (listeners[event] ||= []).push(handler);
+              return Promise.resolve(handler);
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
+    { hosts: HOSTS, devices: DEVICES }
+  );
+  await page.goto('/');
+  await expect(page.getByText('2 hosts')).toBeVisible();
+}
+
+const calls = (page: Page, cmd: string) =>
+  page.evaluate(
+    (cmd) => ((window as unknown as { __calls: Call[] }).__calls ?? []).filter((c) => c.cmd === cmd).map((c) => c.args),
+    cmd
+  );
+
+const fire = (page: Page, event: string, payload: unknown) =>
+  page.evaluate(([event, payload]) => (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire(event as string, payload), [event, payload] as const);
+
+/** Drags `paths` from outside onto the middle of `target` (or a point), optionally
+ *  dropping them. */
+async function dragOnto(
+  page: Page,
+  target: Locator | { x: number; y: number },
+  paths: string[],
+  drop = true
+): Promise<void> {
+  let x: number;
+  let y: number;
+  if ('boundingBox' in target) {
+    const box = (await target.boundingBox())!;
+    [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
+  } else {
+    ({ x, y } = target);
+  }
+  const drag = (type: string, px: number, py: number) =>
+    page.evaluate(
+      ([type, px, py, paths]) =>
+        (window as unknown as { __drag: (t: string, x: number, y: number, p: string[]) => void }).__drag(
+          type as string,
+          px as number,
+          py as number,
+          paths as string[]
+        ),
+      [type, px, py, paths] as const
+    );
+  await drag('enter', x, y);
+  await drag('over', x, y);
+  if (drop) await drag('drop', x, y);
+}
+
+const progress = (sessionId: number, over: Record<string, unknown> = {}) => ({
+  sessionId,
+  waiting: false,
+  fileIndex: 0,
+  fileCount: 1,
+  fileName: 'fw.bin',
+  sent: 0,
+  total: 1000,
+  ...over
+});
+
+async function openTerminal(page: Page, host: string): Promise<Locator> {
+  await page.getByTitle(`sh on ${host}`).click();
+  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
+  return page.locator('.xterm').first();
+}
+
+test('a file dropped on a terminal goes up by ZMODEM, keys stay out of the transfer', async ({ page }) => {
+  await boot(page);
+  const term = await openTerminal(page, 'web-1');
+
+  // Hovering shows where it would go.
+  await dragOnto(page, term, ['/home/me/fw.bin'], false);
+  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with ZMODEM to web-1');
+  await dragOnto(page, term, ['/home/me/fw.bin']);
+  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
+  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
+    { sessionId: 1, protocol: 'zmodem', paths: ['/home/me/fw.bin'] }
+  ]);
+
+  const bar = page.getByRole('status', { name: 'Upload' });
+  await expect(bar).toContainText('Starting ZMODEM…');
+  await fire(page, 'upload-progress', progress(1, { waiting: true, fileName: '', total: 0 }));
+  await expect(bar).toContainText('Waiting for rz…');
+  await fire(page, 'upload-progress', progress(1, { sent: 500 }));
+  await expect(page.getByTestId('upload-text')).toHaveText('fw.bin · 50%');
+
+  // Typing goes nowhere while the protocol owns the stream.
+  await term.click();
+  await page.keyboard.type('ls');
+  await page.waitForTimeout(100);
+  expect(await calls(page, 'terminal_write')).toEqual([]);
+
+  await bar.getByRole('button', { name: 'Cancel' }).click();
+  await expect.poll(() => calls(page, 'upload_cancel')).toEqual([{ sessionId: 1 }]);
+  await fire(page, 'upload-finished', { sessionId: 1, ok: false, message: 'cancelled' });
+  await expect(bar).toHaveCount(0);
+
+  await term.click();
+  await page.keyboard.type('x');
+  await expect.poll(async () => (await calls(page, 'terminal_write')).length).toBeGreaterThan(0);
+});
+
+test('the host setting and the pane override pick the protocol; SFTP leaves the keyboard alone', async ({ page }) => {
+  await boot(page);
+  const term = await openTerminal(page, 'nas');
+  await dragOnto(page, term, ['/a.txt', '/b.txt'], false);
+  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with SFTP to nas');
+  await dragOnto(page, term, ['/a.txt', '/b.txt']);
+  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
+    { sessionId: 1, protocol: 'sftp', paths: ['/a.txt', '/b.txt'] }
+  ]);
+  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Connecting for SFTP…');
+  await term.click();
+  await page.keyboard.type('y');
+  await expect.poll(async () => (await calls(page, 'terminal_write')).length).toBeGreaterThan(0);
+  // The picker is locked while the upload runs.
+  await expect(page.getByLabel('Upload protocol')).toBeDisabled();
+  await fire(page, 'upload-finished', { sessionId: 1, ok: true, message: 'uploaded 2 files (2.0 KB)' });
+
+  // This pane only: switch to ZMODEM and drop again.
+  await page.getByLabel('Upload protocol').selectOption('zmodem');
+  await dragOnto(page, term, ['/c.bin']);
+  await expect.poll(async () => (await calls(page, 'terminal_upload')).at(-1)).toEqual({
+    sessionId: 1,
+    protocol: 'zmodem',
+    paths: ['/c.bin']
+  });
+});
+
+test('a refused drop says why and leaves no progress bar; a drop elsewhere does nothing', async ({ page }) => {
+  await boot(page);
+  const term = await openTerminal(page, 'web-1');
+  await page.evaluate(() => ((window as unknown as { __refuse: string }).__refuse = 'folders cannot be uploaded: photos'));
+  await dragOnto(page, term, ['/home/me/photos']);
+  await expect(page.getByText('Upload: folders cannot be uploaded: photos')).toBeVisible();
+  await expect(page.getByRole('status', { name: 'Upload' })).toHaveCount(0);
+
+  // The sidebar, top left, is not a terminal.
+  await dragOnto(page, { x: 20, y: 120 }, ['/a'], false);
+  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
+  await dragOnto(page, { x: 20, y: 120 }, ['/a']);
+  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
+  expect(await calls(page, 'terminal_upload')).toHaveLength(1);
+});
+
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `upload.spec.ts` 的 3 个用例全部失败（没有遮罩：`toHaveText` / `toBeVisible` 失败）。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q3-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q3-impl.patch && git apply /tmp/q3-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/TerminalPane.svelte b/crates/omnyssh-gui/ui/src/lib/screens/TerminalPane.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/TerminalPane.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/TerminalPane.svelte
@@ -28,8 +28,14 @@
     terminalClose,
     terminalPaste,
     logStop,
-    logStatus
+    logStatus,
+    terminalUpload,
+    uploadCancel
   } from '$lib/ipc/commands';
+  import { blocksInput, endUpload, startUpload, uploads } from '$lib/stores/uploads';
+  import { registerDropTarget } from './dropTargets';
+  import UploadStatus from './UploadStatus.svelte';
+  import Select from '$lib/components/Select.svelte';
   import { shouldFadeTop } from './terminalFade';
   import { chunkBytes, layoutFallback } from './terminalInput';
   import { attachMouseClipboard, copySelection, pasteClipboard } from './terminalClipboard';
@@ -40,7 +46,7 @@
   import { attachWheelZoom } from './terminalZoom';
   import { isMac } from '$lib/platform';
   import type { SplitDir } from './splitLayout';
-  import type { TerminalBytes } from '$lib/bindings';
+  import type { HostUploadProtocolDto, TerminalBytes } from '$lib/bindings';
   import TerminalSearch from './TerminalSearch.svelte';
   import { HIGHLIGHT_LIMIT } from './terminalSearch';
   import { bufferText, formatBytes, logButtonDisabled, sshLogDetail } from './sessionLog';
@@ -99,7 +105,8 @@
   // them; a serialization chain keeps all input strictly in order across events.
   let writeChain: Promise<void> = Promise.resolve();
   function sendInput(bytes: Uint8Array): void {
-    if (termId == null || bytes.length === 0) return;
+    // A ZMODEM upload owns the stream: a keystroke would land inside it.
+    if (termId == null || bytes.length === 0 || blocksInput(get(uploads).get(termId))) return;
     writeChain = writeChain.then(async () => {
       for (const chunk of chunkBytes(bytes)) {
         if (destroyed || termId == null) return;
@@ -360,6 +367,8 @@
     clearTimeout(badgeTimer);
     exitOff?.();
     resizeObserver?.disconnect();
+    // An upload whose session ends gets no upload-finished.
+    endUpload(termId);
     // Idempotent: a remote-exit teardown already dropped this id backend-side (§3.4).
     if (termId != null) void terminalClose(termId).catch(() => {});
     term?.dispose();
@@ -420,6 +429,11 @@
   const titleBtn =
     'term-btn grid h-5 w-5 place-items-center rounded transition ' +
     'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
+  // The upload protocol picker: as wide as its label, otherwise like the buttons.
+  const toolPicker =
+    'term-btn h-6 rounded transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
+  const titlePicker =
+    'term-btn h-5 rounded transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
 
   /** The action's current shortcut, formatted for a title, or empty when unbound. */
   function hint(action: TerminalAction): string {
@@ -468,6 +482,37 @@
     if (termId == null || !logging) return;
     const s = await logStatus(termId).catch(() => null);
     logTitle = s ? `Stop logging (${s.path}, ${formatBytes(s.bytes)})` : '';
+  }
+
+  // Drag-and-drop upload (plan Q): files dropped on the pane go to the host with its
+  // upload protocol, or the one picked here for this pane only.
+  let root = $state<HTMLDivElement>();
+  let protocolOverride = $state<HostUploadProtocolDto | null>(null);
+  const uploadProtocol = $derived<HostUploadProtocolDto>(
+    protocolOverride ?? $hosts.find((h) => h.name === hostName)?.uploadProtocol ?? 'zmodem'
+  );
+  const uploading = $derived(termId != null && $uploads.has(termId));
+  $effect(() => {
+    if (!root) return;
+    return registerDropTarget(root, {
+      label: () => hostName,
+      protocol: () => uploadProtocol,
+      drop: upload
+    });
+  });
+
+  function upload(paths: string[]): void {
+    if (termId == null) {
+      lastError.set('The terminal is still connecting');
+      return;
+    }
+    void startUpload(termId, uploadProtocol, paths, (id, protocol, files) =>
+      terminalUpload(id, protocol as HostUploadProtocolDto, files)
+    );
+  }
+
+  function cancelUpload(): void {
+    if (termId != null) void uploadCancel(termId).catch((e) => lastError.set(`Cancel: ${message(e)}`));
   }
 
   function exportContents(): void {
@@ -514,9 +559,29 @@
   </button>
 {/snippet}
 
+<!-- The protocol files dropped on this pane upload with; changing it lasts for this
+     pane only. -->
+{#snippet protocolPicker(cls: string, style: string)}
+  <Select
+    class="{cls} cursor-pointer bg-transparent pl-1.5 text-[11px] font-medium disabled:cursor-not-allowed disabled:opacity-40"
+    {style}
+    aria-label="Upload protocol"
+    title="Files dropped here upload with this"
+    disabled={uploading}
+    bind:value={
+      () => uploadProtocol,
+      (v) => (protocolOverride = v as HostUploadProtocolDto)
+    }
+  >
+    <option value="zmodem">ZMODEM</option>
+    <option value="sftp">SFTP</option>
+  </Select>
+{/snippet}
+
 <!-- Clicking anywhere in the pane, or xterm's textarea taking focus, makes it the
      tab's focused pane. -->
 <div
+  bind:this={root}
   class="group relative h-full w-full {framed ? 'p-0.5' : ''}"
   role="presentation"
   onpointerdown={onFocus}
@@ -557,6 +622,7 @@
         >
           {title || hostName}
         </span>
+        {@render protocolPicker(titlePicker, '')}
         {@render logButtons(titleBtn, '', 13)}
         <button type="button" class={titleBtn} title="Split right{hint('splitRight')}" aria-label="Split right" onclick={() => onSplit('row')}>
           <Icon name="splitRight" size={13} />
@@ -576,6 +642,7 @@
       bind:this={toolbar}
       class="peer absolute right-1.5 top-1.5 z-10 flex gap-1 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100"
     >
+      {@render protocolPicker(toolPicker, toolBtnStyle)}
       {@render logButtons(toolBtn, toolBtnStyle, 14)}
       <button type="button" class={toolBtn} style={toolBtnStyle} title="Split right{hint('splitRight')}" aria-label="Split right" onclick={() => onSplit('row')}>
         <Icon name="splitRight" size={14} />
@@ -607,6 +674,7 @@
       <TerminalSearch addon={searchAddon} focusToken={searchFocus} onClose={closeSearch} />
     </div>
   {/if}
+  <UploadStatus {root} sessionId={termId} onCancel={cancelUpload} />
   {#if sizeBadge !== null}
     <div
       class="pointer-events-none absolute left-1/2 top-1/2 z-30 -translate-x-1/2 -translate-y-1/2 rounded-md bg-surface-raised px-3 py-1.5 text-sm font-medium text-fg shadow-soft"
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/UploadStatus.svelte b/crates/omnyssh-gui/ui/src/lib/screens/UploadStatus.svelte
new file mode 100644
--- /dev/null
+++ b/crates/omnyssh-gui/ui/src/lib/screens/UploadStatus.svelte
@@ -0,0 +1,68 @@
+<script lang="ts">
+  // A view's drag-and-drop upload UI (plan Q): the overlay while files are dragged over
+  // it, and the progress bar with Cancel while its upload runs. Placed inside the view's
+  // root, which must be `position: relative`.
+  import { dropHover } from './dropTargets';
+  import { dropError, dropNote, progressLine, protocolName, uploads } from '$lib/stores/uploads';
+
+  let {
+    root,
+    sessionId,
+    onCancel
+  }: {
+    /** The view's registered drop target. */
+    root: Element | undefined;
+    /** The backend session id; undefined while connecting. */
+    sessionId: number | undefined;
+    onCancel: () => void;
+  } = $props();
+
+  const hover = $derived($dropHover && root && $dropHover.el === root ? $dropHover : null);
+  const refusal = $derived(hover ? dropError(hover.protocol, hover.count) : null);
+  const note = $derived(hover ? dropNote(hover.protocol, hover.count) : null);
+  const view = $derived(sessionId == null ? undefined : $uploads.get(sessionId));
+  const line = $derived(view ? progressLine(view) : null);
+</script>
+
+{#if hover}
+  <div
+    class="pointer-events-none absolute inset-0 z-40 grid place-items-center rounded"
+    style="background: color-mix(in srgb, var(--bg) 70%, transparent);"
+    data-testid="drop-overlay"
+  >
+    <div class="max-w-[80%] rounded-lg bg-surface-raised px-4 py-3 text-center text-sm text-fg shadow-soft">
+      {#if refusal}
+        <p class="text-status-crit">{refusal}</p>
+      {:else}
+        <p>Drop to upload with {protocolName(hover.protocol)} to {hover.label}</p>
+        {#if note}<p class="mt-1 text-xs text-muted">{note}</p>{/if}
+      {/if}
+    </div>
+  </div>
+{/if}
+
+{#if view && line}
+  <div
+    class="absolute inset-x-1.5 bottom-1.5 z-30 flex items-center gap-2 rounded-md bg-surface-raised px-2.5 py-1.5 text-xs text-fg shadow-soft"
+    role="status"
+    aria-label="Upload"
+  >
+    <span class="shrink-0 font-medium">{protocolName(view.protocol)}</span>
+    <div class="min-w-0 flex-1">
+      <div class="truncate" data-testid="upload-text">{line.text}</div>
+      <div class="mt-1 h-1 overflow-hidden rounded-full bg-surface-inset">
+        <div
+          class="h-full rounded-full bg-accent transition-[width] {line.fraction === null ? 'w-1/3 animate-pulse' : ''}"
+          style={line.fraction === null ? '' : `width: ${line.fraction * 100}%`}
+        ></div>
+      </div>
+    </div>
+    <button
+      type="button"
+      class="shrink-0 rounded px-2 py-0.5 text-xs text-muted transition hover:bg-surface hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
+      onclick={onCancel}
+    >
+      Cancel
+    </button>
+  </div>
+{/if}
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; npm run check 2>&1 | tail -1; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `upload.spec.ts` 3 个用例通过；svelte-check 无错误。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/screens/UploadStatus.svelte crates/omnyssh-gui/ui/src/lib/screens/TerminalPane.svelte crates/omnyssh-gui/ui/e2e/upload.spec.ts crates/omnyssh-gui/ui/e2e/terminal.spec.ts
git commit -m "feat(ui): drop files on a terminal pane to upload them"
```

---

### Task 4: 串口标签页

**Files:** Modify `ui/src/lib/screens/SerialView.svelte`、`ui/src/lib/stores/sessions.ts`、`ui/src/lib/screens/Dashboard.svelte`、`ui/e2e/upload.spec.ts`

新增 2 个用例：从已保存设备打开的标签页按设备的协议（YMODEM）上传，多文件时遮罩提示 U-Boot 只收第一个；改成 XMODEM 后多文件被拒绝；只接收模式的标签页不接受拖放。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q4-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q4-test.patch && git apply /tmp/q4-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/upload.spec.ts b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
--- a/crates/omnyssh-gui/ui/e2e/upload.spec.ts
+++ b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
@@ -252,3 +252,41 @@
   expect(await calls(page, 'terminal_upload')).toHaveLength(1);
 });
 
+test('a saved device uploads with its protocol; XMODEM takes one file', async ({ page }) => {
+  await boot(page);
+  await page.getByTitle('term on board').click();
+  const view = page.locator('.xterm').first();
+  await expect(view).toBeVisible();
+  await expect(page.getByLabel('Upload protocol')).toHaveValue('ymodem');
+
+  await dragOnto(page, view, ['/u-boot.bin', '/env.bin'], false);
+  await expect(page.getByTestId('drop-overlay')).toContainText('Drop to upload with YMODEM to board');
+  await expect(page.getByTestId('drop-overlay')).toContainText('U-Boot takes only the first file');
+  await dragOnto(page, view, ['/u-boot.bin']);
+  await expect.poll(() => calls(page, 'serial_upload')).toEqual([
+    { sessionId: 1, protocol: 'ymodem', paths: ['/u-boot.bin'] }
+  ]);
+  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Starting YMODEM…');
+  await fire(page, 'upload-progress', progress(1, { waiting: true, fileName: '', total: 0 }));
+  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Waiting for the receiver…');
+  await fire(page, 'upload-finished', { sessionId: 1, ok: true, message: 'uploaded u-boot.bin (1 B)' });
+
+  await page.getByLabel('Upload protocol').selectOption('xmodem');
+  await dragOnto(page, view, ['/a.bin', '/b.bin'], false);
+  await expect(page.getByTestId('drop-overlay')).toHaveText('XMODEM sends one file at a time');
+  await dragOnto(page, view, ['/a.bin', '/b.bin']);
+  await expect(page.getByText('XMODEM sends one file at a time')).toBeVisible();
+  expect(await calls(page, 'serial_upload')).toHaveLength(1);
+});
+
+test('a receive-only serial tab takes no drops', async ({ page }) => {
+  await boot(page);
+  await page.getByTitle('monitor on board').click();
+  const view = page.locator('.xterm').first();
+  await expect(view).toBeVisible();
+  await expect(page.getByLabel('Upload protocol')).toHaveCount(0);
+  await dragOnto(page, view, ['/a.bin']);
+  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
+  expect(await calls(page, 'serial_upload')).toEqual([]);
+});
+
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `a saved device uploads with its protocol; XMODEM takes one file` 失败（没有协议下拉框：`toHaveValue` 失败）。`a receive-only serial tab takes no drops` 这时就能通过，它守的是"只接收模式不接拖放"这条行为。其余 3 个通过。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q4-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q4-impl.patch && git apply /tmp/q4-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte b/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte
@@ -159,7 +159,12 @@
   const deviceNames = $derived(devices.map((d) => d.name));
 
   function openDevice(device: SerialDeviceDto, mode: SerialMode): void {
-    spawnSession('serial', device.name, { config: device.config, mode, enter: device.enter });
+    spawnSession('serial', device.name, {
+      config: device.config,
+      mode,
+      enter: device.enter,
+      upload: device.uploadProtocol
+    });
   }
 
   async function submitDevice(device: SerialDeviceDto): Promise<void> {
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/SerialView.svelte b/crates/omnyssh-gui/ui/src/lib/screens/SerialView.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/SerialView.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/SerialView.svelte
@@ -16,7 +16,19 @@
   import { sessions, type Session } from '$lib/stores/sessions';
   import { lastError } from '$lib/stores/notifications';
   import { dialogs } from '$lib/stores/dialogs';
-  import { serialOpen, serialWrite, serialClose, logStop, logStatus } from '$lib/ipc/commands';
+  import {
+    serialOpen,
+    serialWrite,
+    serialClose,
+    logStop,
+    logStatus,
+    serialUpload,
+    uploadCancel
+  } from '$lib/ipc/commands';
+  import { blocksInput, endUpload, startUpload, uploads } from '$lib/stores/uploads';
+  import { registerDropTarget } from './dropTargets';
+  import UploadStatus from './UploadStatus.svelte';
+  import Select from '$lib/components/Select.svelte';
   import { sessionLogs, startSessionLog, exportText } from '$lib/stores/sessionLogs';
   import { attachMouseClipboard, copySelection, pasteClipboard } from './terminalClipboard';
   import { chunkBytes } from './terminalInput';
@@ -25,7 +37,7 @@
   import { quickCommandBytes } from '$lib/stores/quickCommands';
   import { logTimestamps, serialTimestamps, terminalFontSize } from '$lib/stores/terminalPrefs';
   import { ByteHistory, SerialFormatter, mapEnter, type SerialDisplay } from './serialFormat';
-  import type { SerialExitDto, TerminalBytes } from '$lib/bindings';
+  import type { SerialExitDto, SerialUploadProtocolDto, TerminalBytes } from '$lib/bindings';
   import TerminalSearch from './TerminalSearch.svelte';
   import { HIGHLIGHT_LIMIT } from './terminalSearch';
   import QuickCommandBar from './QuickCommandBar.svelte';
@@ -96,7 +108,8 @@
   // Chunked and serialized like TerminalView's input, so a large paste stays in order.
   let writeChain: Promise<void> = Promise.resolve();
   function sendInput(bytes: Uint8Array): void {
-    if (serialId == null || bytes.length === 0) return;
+    // A ZMODEM/YMODEM/XMODEM upload owns the line: a keystroke would land inside it.
+    if (serialId == null || bytes.length === 0 || blocksInput(get(uploads).get(serialId))) return;
     writeChain = writeChain.then(async () => {
       for (const chunk of chunkBytes(bytes)) {
         if (destroyed || serialId == null) return;
@@ -230,6 +243,8 @@
       if (destroyed || mine !== attempt) return;
       // Keep the tab and its output: the log up to the failure is what matters.
       failed = true;
+      // An upload on a port that went away gets no upload-finished.
+      endUpload(serialId);
       canSend = false;
       sessions.setStatus(session.id, 'failed');
       lastError.set(`${opts.config.port}: ${msg.error}`);
@@ -262,6 +277,7 @@
     reconnecting = true;
     const old = serialId;
     serialId = undefined;
+    endUpload(old);
     sessions.setStatus(session.id, 'connecting');
     try {
       // Release the dead port first, so the reopen isn't refused as busy.
@@ -402,10 +418,41 @@
     zoomOff?.();
     clearTimeout(badgeTimer);
     resizeObserver?.disconnect();
+    endUpload(serialId);
     if (serialId != null) void serialClose(serialId).catch(() => {});
     term?.dispose();
     term = undefined;
   });
+
+  // Drag-and-drop upload (plan Q), in terminal mode only: a receive-only tab sends
+  // nothing. The protocol picked in the header lasts for this tab.
+  let dropRoot = $state<HTMLDivElement>();
+  let uploadProtocol = $state<SerialUploadProtocolDto>(opts.upload ?? 'zmodem');
+  const uploading = $derived(serialId != null && $uploads.has(serialId));
+  $effect(() => {
+    if (!dropRoot || opts.mode !== 'terminal') return;
+    return registerDropTarget(dropRoot, {
+      label: () => session.hostName,
+      protocol: () => uploadProtocol,
+      drop: upload
+    });
+  });
+
+  function upload(paths: string[]): void {
+    if (serialId == null || !canSend) {
+      lastError.set(`${opts.config.port} is not open`);
+      return;
+    }
+    void startUpload(serialId, uploadProtocol, paths, (id, protocol, files) =>
+      serialUpload(id, protocol as SerialUploadProtocolDto, files)
+    );
+  }
+
+  function cancelUpload(): void {
+    if (serialId != null) {
+      void uploadCancel(serialId).catch((e) => lastError.set(`Cancel: ${message(e)}`));
+    }
+  }
 
   $effect(() => {
     if (active && ready) {
@@ -476,6 +523,19 @@
         Time
       </button>
     {/if}
+    {#if opts.mode === 'terminal'}
+      <Select
+        class="cursor-pointer rounded-full border border-default bg-transparent py-0.5 pl-3 text-xs text-muted transition hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus disabled:cursor-not-allowed disabled:opacity-40"
+        aria-label="Upload protocol"
+        title="Files dropped here upload with this"
+        disabled={uploading}
+        bind:value={uploadProtocol}
+      >
+        <option value="zmodem">ZMODEM</option>
+        <option value="ymodem">YMODEM</option>
+        <option value="xmodem">XMODEM</option>
+      </Select>
+    {/if}
     <Button variant="ghost" title="Clear the output" onclick={clear}>Clear</Button>
     <button
       type="button"
@@ -496,7 +556,11 @@
       Export
     </Button>
   </div>
-  <div class="relative min-h-0 flex-1 px-2 pt-2" style="background: {$terminalColors.background}">
+  <div
+    bind:this={dropRoot}
+    class="relative min-h-0 flex-1 px-2 pt-2"
+    style="background: {$terminalColors.background}"
+  >
     <div bind:this={container} class="h-full w-full"></div>
     {#if searchOpen && searchAddon}
       <div class="absolute right-3 top-2 z-20 max-w-[calc(100%-1.5rem)]">
@@ -511,6 +575,7 @@
         {sizeBadge} px
       </div>
     {/if}
+    <UploadStatus root={dropRoot} sessionId={serialId} onCancel={cancelUpload} />
   </div>
   <QuickCommandBar
     enabled={canSend}
diff --git a/crates/omnyssh-gui/ui/src/lib/stores/sessions.ts b/crates/omnyssh-gui/ui/src/lib/stores/sessions.ts
--- a/crates/omnyssh-gui/ui/src/lib/stores/sessions.ts
+++ b/crates/omnyssh-gui/ui/src/lib/stores/sessions.ts
@@ -1,6 +1,6 @@
 import { writable } from 'svelte/store';
 import type { Status } from '$lib/theme';
-import type { SerialConfigDto } from '$lib/bindings';
+import type { SerialConfigDto, SerialUploadProtocolDto } from '$lib/bindings';
 import type { EnterKey } from '$lib/screens/serialFormat';
 
 // The open terminal/SFTP/serial tabs (tech-gui.md §2, §3.5). Spawners append a row
@@ -26,6 +26,8 @@
   config: SerialConfigDto;
   mode: SerialMode;
   enter: EnterKey;
+  /** How files dropped on the tab upload; a saved device's setting, else ZMODEM. */
+  upload?: SerialUploadProtocolDto;
 }
 
 export interface Session {
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; npm run check 2>&1 | tail -1; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: `upload.spec.ts` 5 个用例通过；svelte-check 无错误。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/screens/SerialView.svelte crates/omnyssh-gui/ui/src/lib/stores/sessions.ts crates/omnyssh-gui/ui/src/lib/screens/Dashboard.svelte crates/omnyssh-gui/ui/e2e/upload.spec.ts
git commit -m "feat(ui): drop files on a serial tab to upload them"
```

---

### Task 5: 表单

**Files:** Modify `ui/src/lib/screens/hostForm.ts`、`hostForm.test.ts`、`HostEditor.svelte`、`serialForm.ts`、`serialForm.test.ts`、`SerialDeviceEditor.svelte`、`ui/e2e/upload.spec.ts`

新增 2 个 e2e 用例（主机表单设 SFTP 和目录、设备表单改协议）和几个单元测试。

- [ ] **Step 1: 先加测试**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q5-test.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q5-test.patch && git apply /tmp/q5-test.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/e2e/upload.spec.ts b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
--- a/crates/omnyssh-gui/ui/e2e/upload.spec.ts
+++ b/crates/omnyssh-gui/ui/e2e/upload.spec.ts
@@ -290,3 +290,27 @@
   expect(await calls(page, 'serial_upload')).toEqual([]);
 });
 
+test('the host editor sets SFTP and its folder', async ({ page }) => {
+  await boot(page);
+  await page.getByRole('button', { name: 'Edit web-1' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit host' });
+  await expect(editor.getByLabel('Upload folder')).toHaveCount(0);
+  await editor.getByLabel('Drag-and-drop upload').selectOption('sftp');
+  await editor.getByLabel('Upload folder').fill('/srv/in');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect.poll(async () => (await calls(page, 'save_host'))[0]).toMatchObject({
+    input: { name: 'web-1', uploadProtocol: 'sftp', uploadDir: '/srv/in' }
+  });
+});
+
+test('the serial device editor keeps the upload protocol', async ({ page }) => {
+  await boot(page);
+  await page.getByRole('button', { name: 'Edit board' }).click();
+  const editor = page.getByRole('dialog', { name: 'Edit serial device' });
+  await expect(editor.getByLabel('Drag-and-drop upload')).toHaveValue('ymodem');
+  await editor.getByLabel('Drag-and-drop upload').selectOption('xmodem');
+  await editor.getByRole('button', { name: 'Save' }).click();
+  await expect.poll(async () => (await calls(page, 'save_serial_device'))[0]).toMatchObject({
+    device: { name: 'board', uploadProtocol: 'xmodem' }
+  });
+});
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.test.ts
@@ -141,7 +141,9 @@
       localForwards: [],
       tunnelAutostart: false,
       forwardAgent: false,
-      monitorPort: undefined
+      monitorPort: undefined,
+      uploadProtocol: 'zmodem',
+      uploadDir: undefined
     });
   });
 });
@@ -325,3 +327,21 @@
     expect(form.password).toBe('');
   });
 });
+
+describe('formToInput — drag-and-drop upload', () => {
+  it('always sends the protocol, and the folder only for SFTP', () => {
+    const sftp = formToInput(fields({ name: 'nas', hostname: 'h', uploadProtocol: 'sftp', uploadDir: ' ~/drop ' }));
+    expect(sftp.ok && [sftp.input.uploadProtocol, sftp.input.uploadDir]).toEqual(['sftp', '~/drop']);
+    const blank = formToInput(fields({ name: 'nas', hostname: 'h', uploadProtocol: 'sftp', uploadDir: '  ' }));
+    expect(blank.ok && blank.input.uploadDir).toBeUndefined();
+    const zmodem = formToInput(fields({ name: 'nas', hostname: 'h', uploadDir: '/srv' }));
+    expect(zmodem.ok && [zmodem.input.uploadProtocol, zmodem.input.uploadDir]).toEqual(['zmodem', undefined]);
+  });
+
+  it('seeds the edit form from the host, ZMODEM when it says nothing', () => {
+    const f = formFromHost(host({ uploadProtocol: 'sftp', uploadDir: '/srv/in' }));
+    expect([f.uploadProtocol, f.uploadDir]).toEqual(['sftp', '/srv/in']);
+    expect(formFromHost(host({})).uploadProtocol).toBe('zmodem');
+    expect(emptyForm().uploadProtocol).toBe('zmodem');
+  });
+});
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/serialForm.test.ts b/crates/omnyssh-gui/ui/src/lib/screens/serialForm.test.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/serialForm.test.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/serialForm.test.ts
@@ -21,7 +21,8 @@
     flowControl: 'none'
   },
   enter: 'crlf',
-  notes: 'bench board'
+  notes: 'bench board',
+  uploadProtocol: 'ymodem'
 };
 
 describe('lineConfig', () => {
@@ -65,6 +66,12 @@
     });
   });
 
+  it('read a device saved before uploads as ZMODEM', () => {
+    const { uploadProtocol: _, ...old } = device;
+    expect(fieldsFromDevice(old).uploadProtocol).toBe('zmodem');
+    expect(emptyDeviceFields().uploadProtocol).toBe('zmodem');
+  });
+
   it('drop blank notes', () => {
     const result = deviceFromFields({ ...fieldsFromDevice(device), notes: '  ' }, []);
     expect(result.ok && result.device.notes).toBeNull();
```

- [ ] **Step 2: 确认测试失败**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run src/lib/screens/hostForm.test.ts src/lib/screens/serialForm.test.ts 2>&1 | grep -E "Tests |×" | head -8; npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: vitest 里 5 个失败（`round-trip a device`、`read a device saved before uploads as ZMODEM`、`round-trips the observable fields back through formToInput`，以及两个 `drag-and-drop upload` 用例）；e2e 里两个表单用例失败，其余 5 个通过。

- [ ] **Step 3: 实现**

用 Write 工具把下面的补丁**原样**存成 `/tmp/q5-impl.patch`（不要改动空格和空行），然后在仓库根目录执行 `git apply --check /tmp/q5-impl.patch && git apply /tmp/q5-impl.patch`。

```diff
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte b/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/HostEditor.svelte
@@ -184,6 +184,29 @@
       {#if fields.monitoring === 'tcpPort'}
         <p class="text-xs text-faint">Checks the port only — no login, and no metrics on the card.</p>
       {/if}
+
+      <div class="grid grid-cols-2 gap-3">
+        <label class={label}>
+          <span>Drag-and-drop upload</span>
+          <Select bind:value={fields.uploadProtocol} class={field}>
+            <option value="zmodem">ZMODEM (rz)</option>
+            <option value="sftp">SFTP</option>
+          </Select>
+        </label>
+        {#if fields.uploadProtocol === 'sftp'}
+          <label class={label}>
+            <span>Upload folder</span>
+            <input bind:value={fields.uploadDir} class={field} placeholder="~" />
+          </label>
+        {/if}
+      </div>
+      <p class="text-xs text-faint">
+        {#if fields.uploadProtocol === 'sftp'}
+          Files dropped on a terminal go to this folder over SFTP; the shell is left alone.
+        {:else}
+          Files dropped on a terminal are sent to <span class="font-mono">rz</span> in its shell — the host needs lrzsz.
+        {/if}
+      </p>
 
       <div class="flex items-center justify-between gap-4 border-t border-default pt-3.5">
         <div class="min-w-0">
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/SerialDeviceEditor.svelte b/crates/omnyssh-gui/ui/src/lib/screens/SerialDeviceEditor.svelte
--- a/crates/omnyssh-gui/ui/src/lib/screens/SerialDeviceEditor.svelte
+++ b/crates/omnyssh-gui/ui/src/lib/screens/SerialDeviceEditor.svelte
@@ -5,6 +5,7 @@
   import type { SerialDeviceDto } from '$lib/bindings';
   import Modal from '$lib/components/Modal.svelte';
   import { Button } from '$lib/theme';
+  import Select from '$lib/components/Select.svelte';
   import SerialLineFields from './SerialLineFields.svelte';
   import { deviceFromFields, type SerialDeviceFields } from './serialForm';
 
@@ -88,6 +89,14 @@
         <SerialLineFields bind:fields />
       </div>
       <label class={label}>
+        <span>Drag-and-drop upload</span>
+        <Select bind:value={fields.uploadProtocol} class={field}>
+          <option value="zmodem">ZMODEM (rz)</option>
+          <option value="ymodem">YMODEM (loady, rb)</option>
+          <option value="xmodem">XMODEM (loadx, rx)</option>
+        </Select>
+      </label>
+      <label class={label}>
         <span>Notes</span>
         <input bind:value={fields.notes} class={field} placeholder="Optional" />
       </label>
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/hostForm.ts
@@ -3,7 +3,13 @@
 // validation mirrors the TUI's `HostForm::to_host` (crates/omnyssh/src/app/host.rs)
 // so both frontends produce the same `hosts.toml` shape and error messages.
 
-import type { HostDto, HostInputDto, LocalForwardDto, MonitorModeDto } from '$lib/bindings';
+import type {
+  HostDto,
+  HostInputDto,
+  HostUploadProtocolDto,
+  LocalForwardDto,
+  MonitorModeDto
+} from '$lib/bindings';
 
 /** One port-forward row as typed: listen on `[address:]port` here, reach
  *  `remoteHost:remotePort` from the host. */
@@ -29,6 +35,10 @@
   forwards: ForwardRow[];
   tunnelAutostart: boolean;
   forwardAgent: boolean;
+  /** How files dropped on this host's terminals upload. */
+  uploadProtocol: HostUploadProtocolDto;
+  /** Where SFTP uploads go: blank means the home folder. Only read for `sftp`. */
+  uploadDir: string;
 }
 
 export function emptyForm(): HostFormFields {
@@ -47,7 +57,9 @@
     monitorPort: '',
     forwards: [],
     tunnelAutostart: false,
-    forwardAgent: false
+    forwardAgent: false,
+    uploadProtocol: 'zmodem',
+    uploadDir: ''
   };
 }
 
@@ -83,7 +95,9 @@
     monitorPort: h.monitorPort == null ? '' : String(h.monitorPort),
     forwards: h.localForwards.map(rowFromForward),
     tunnelAutostart: h.tunnelAutostart,
-    forwardAgent: h.forwardAgent
+    forwardAgent: h.forwardAgent,
+    uploadProtocol: h.uploadProtocol ?? 'zmodem',
+    uploadDir: h.uploadDir ?? ''
   };
 }
 
@@ -221,7 +235,11 @@
       localForwards,
       // The switch hides with the last row; a flag nobody can see must not linger.
       tunnelAutostart: f.tunnelAutostart && localForwards.length > 0,
-      forwardAgent: f.forwardAgent
-    }
-  };
-}
+      forwardAgent: f.forwardAgent,
+      // Always sent: an omitted protocol means "keep" to the backend, and takes the
+      // folder with it. A folder nobody can see (ZMODEM) is cleared.
+      uploadProtocol: f.uploadProtocol,
+      uploadDir: f.uploadProtocol === 'sftp' ? f.uploadDir.trim() || undefined : undefined
+    }
+  };
+}
diff --git a/crates/omnyssh-gui/ui/src/lib/screens/serialForm.ts b/crates/omnyssh-gui/ui/src/lib/screens/serialForm.ts
--- a/crates/omnyssh-gui/ui/src/lib/screens/serialForm.ts
+++ b/crates/omnyssh-gui/ui/src/lib/screens/serialForm.ts
@@ -7,6 +7,7 @@
   ParityDto,
   SerialConfigDto,
   SerialDeviceDto,
+  SerialUploadProtocolDto,
   StopBitsDto
 } from '$lib/bindings';
 
@@ -23,6 +24,8 @@
 export interface SerialDeviceFields extends SerialLineFields {
   name: string;
   notes: string;
+  /** How files dropped on the device's tab upload. */
+  uploadProtocol: SerialUploadProtocolDto;
 }
 
 /** 115200 8N1, no flow control, Enter sends CR: what almost every MCU boots with. */
@@ -39,7 +42,7 @@
 }
 
 export function emptyDeviceFields(): SerialDeviceFields {
-  return { ...emptyLineFields(), name: '', notes: '' };
+  return { ...emptyLineFields(), name: '', notes: '', uploadProtocol: 'zmodem' };
 }
 
 /** Digits only, 1..u32::MAX (the backend takes a u32). */
@@ -74,7 +77,8 @@
     stopBits: d.config.stopBits,
     flow: d.config.flowControl,
     enter: d.enter,
-    notes: d.notes ?? ''
+    notes: d.notes ?? '',
+    uploadProtocol: d.uploadProtocol ?? 'zmodem'
   };
 }
 
@@ -92,7 +96,13 @@
   const notes = f.notes.trim();
   return {
     ok: true,
-    device: { name, config, enter: f.enter as LineEndingDto, notes: notes || null }
+    device: {
+      name,
+      config,
+      enter: f.enter as LineEndingDto,
+      notes: notes || null,
+      uploadProtocol: f.uploadProtocol
+    }
   };
 }
 
```

- [ ] **Step 4: 跑测试**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npx vitest run 2>&1 | grep -E "Tests |×"; npx playwright test e2e/upload.spec.ts --reporter=line 2>&1 | grep -E "passed|failed|^ +[0-9]+\) "; npm run check 2>&1 | tail -1; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: vitest 全部通过（470 个）；`upload.spec.ts` 7 个用例通过；svelte-check 无错误。

- [ ] **Step 5: Commit**

```bash
git add crates/omnyssh-gui/ui/src/lib/screens crates/omnyssh-gui/ui/e2e/upload.spec.ts
git commit -m "feat(ui): upload protocol fields in the host and serial device forms"
```

---

### Task 6: 全量检查和手动验证

- [ ] **Step 1: 全量检查**

```bash
cd crates/omnyssh-gui/ui && nohup sh -c 'npm run check 2>&1 | tail -1; npx vitest run 2>&1 | grep -E "Tests "; for i in 1 2; do npx playwright test --reporter=line 2>&1 | grep -E "passed|failed|flaky|^ +[0-9]+\\) "; done; echo EXIT=$?' > /tmp/q.log 2>&1 &
```
然后反复 `tail -20 /tmp/q.log`，直到出现 `EXIT=`（命令结束后回到仓库根目录：`cd ../../..`）。

Expected: svelte-check `0 ERRORS 0 WARNINGS`；vitest 470 个通过；两遍 Playwright 都是 104 个通过（这是计划原始代码的数字；执行中审查补了测试，最终是 vitest 479 个、Playwright 109 个）。如果某个原有用例偶发失败，单独重跑它（`npx playwright test e2e/<文件>:<行号> --repeat-each=5`）并在报告里写明，不要改代码掩盖。

- [ ] **Step 2: 手动验证清单（需要人在真实应用里做，执行者只需在报告里列出来）**

在仓库根目录 `cargo tauri dev`（或 `cargo run -p omnyssh-gui`）启动应用：
1. 打开一个装了 lrzsz 的主机的终端，把一个文件拖到终端上：经过时出现遮罩；松手后终端里不出现 `rz` 的帧乱码；进度条走完；终端打印 `[upload] uploaded …`；文件出现在 shell 的当前目录。
2. 上传中按键没有反应；点 `Cancel` 后终端打印 `[upload] cancelled`，shell 恢复可用。
3. 拖到没有 `rz` 的主机：约 25 秒后打印 `[upload] failed: the device did not answer rz (is lrzsz installed?)`，之前 shell 的 `command not found` 也显示出来。
4. 在主机表单里改成 SFTP 并设目录，拖文件：第一次会建立 SFTP 连接（可能要输密码），文件出现在目录里，终端打印结果行，期间可以继续打字。
5. 分屏后拖到右边的窗格：传到右边窗格的会话。
6. 串口（如果有开发板）：U-Boot 里执行 `loady`，在串口标签页把协议改成 YMODEM 后拖入 `.bin`，进度条走完，U-Boot 报告收到的大小。
7. Windows 和 macOS 上各试一次拖放坐标（高 DPI 屏幕）：遮罩出现在鼠标所在的窗格上。
8. 点 Cancel 后不点终端直接打字：按键进入 shell（单窗格和分屏各试一次）。
9. Windows 缩放 125%/150% 的屏幕上拖放，再把窗口拖到 100% 的屏幕上重试；macOS Retina 屏上试，包括窗口一部分在屏幕外时。
10. Windows：拖 Outlook 附件、资源管理器里 zip 内的文件、浏览器里的图片：没有遮罩、没有反应（WebView2 对不带文件列表的拖动不发事件），记为已知行为。
11. Linux：拖进窗口、不松手移出、再移回：遮罩正确消失和出现，移出后没有残留；从文件管理器拖入路径带空格或中文的文件。
12. 拖过分屏分隔线、窗格标题栏、悬停工具栏、打开的查找框：遮罩跟着鼠标下的窗格走，松在标题栏上传给该窗格。（分隔线上没有遮罩、松手无反应，属于已知行为。）
13. 打开设置或密码对话框时拖过窗口：没有遮罩，松手无反应。
14. 终端还在连接（慢主机、或密码对话框开着）时拖入：遮罩显示 `The terminal is still connecting`，松手无反应。
15. 密码登录的主机用 SFTP：拖入后在密码对话框点取消：进度条消失，状态栏显示原因；再试一次，在 `Connecting for SFTP…` 时点 Cancel：不显示错误。
16. YMODEM 上传中拔掉 USB 串口，再重连：进度条消失，可以打字，新的拖放能重新开始。
17. 在后台标签页里上传，切走再切回：进度条正常；上传中关掉这个标签页：不留残留、不报错。

- [ ] **Step 3: 如果 Step 1 有失败**

按失败信息修，修完重跑 Step 1，再单独提交（`fix(ui): …`）。没有失败就跳过。
