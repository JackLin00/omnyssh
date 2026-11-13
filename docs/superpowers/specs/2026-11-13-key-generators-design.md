# 密钥生成器 — 设计

日期：2026-11-13　分支：`feat/serial-port`

## 目标
有一类设备，每台的 SSH 登录私钥都不一样，私钥是由一份共用的凭证（含 `master_key`）加上每台设备的 MAC 和 revision，用一段 JS 算法（用到 Web Crypto）算出来的。用户写好这段代码、填好凭证，给每台设备填上 MAC 和 revision，连接时应用自动算出私钥并用它登录。

## 已与用户确认
- 代码算出的是 **SSH 私钥全文**，不是密码，也不是种子。
- 一个生成器（代码 + 凭证）给**多台设备共用**；每台主机只填自己的 **MAC** 和 **revision**。
- 接口形如 `await generateKey({ credential, mac, revision }, '<主机地址>')`，返回一个对象，应用取其中的 `privateKey`。
- 代码用到 Web Crypto（`crypto.subtle`）。
- 采用方案 B：**连接时生成，私钥只放内存**，不写磁盘。
- 功能必须**主动启用**（总开关，默认关闭），并且**不影响原有的登录方式**：生成的密钥只是多试一次，任何失败都继续走原来的流程。

## 配置
- 新文件 `key_generators.toml`（和 `hosts.toml` 放在同一个目录）：
  ```toml
  enabled = false          # 总开关，默认关闭

  [[generators]]
  name = "vendor-a"
  code = """
  async function generateKey(deviceInfo, host) { ... }
  """
  credential = '{"master_key": "...", ...}'   # JSON 文本
  ```
  - 名字唯一，不能为空。`credential` 必须是合法的 JSON（可以为空，为空时传 `null`）。
  - 文件不存在等于 `enabled = false`、没有生成器。
  - 凭证含有 `master_key`，文件和 `hosts.toml` 一样，以 0600 权限写入。
- `Host`（`omnyssh-core/src/ssh/client.rs`）新增三个字段，为空时都不序列化：
  - `key_generator: Option<String>`：生成器名字。
  - `device_mac: Option<String>`：设备 MAC，原样传给代码，不做格式检查。
  - `device_revision: Option<u32>`：设备 revision；没填时不放进 deviceInfo，由代码自己按默认值处理。
- 复制主机时，这三项一起复制。
- 编辑主机时，表单里没有显示的字段保留原值；TUI 编辑时同样原样保留。

## 代码接口
应用这样调用生成器代码里定义的 `generateKey`：
```js
const keys = await generateKey(
  {
    credential,          // 生成器的凭证，JSON.parse 之后的对象
    mac: '30:ae:7b:eb:17:82',
    revision: 1,         // 主机没填时不放这个键
  },
  '192.168.11.1'         // 主机的 Hostname / IP
);
// keys = { privateKey, publicKey, fingerprint, mac, revision, algorithm }
```
- `generateKey` 可以是 async，也可以是同步函数。可以用 `crypto.subtle`、`TextEncoder` 等标准 API。
- 应用只用 `keys.privateKey`：必须是以 `-----BEGIN` 开头的私钥文本，OpenSSH 或 PEM 格式，RSA、ECDSA、Ed25519 均可，不能带密码。其它字段不参与登录，只有 `fingerprint` 会在测试时拿来对照。
- 没有 `privateKey`、它不是私钥文本、抛出异常、超过 10 秒，都算生成失败。

## 运行环境
- 在前端的独立 Web Worker 里运行。Worker 由"生成器代码 + 一小段调用包装"拼成的 Blob 创建。每次运行都新开一个，结束或超时就 `terminate()`。
- Worker 访问不到应用的内部接口（`__TAURI_INTERNALS__` 只在主窗口里）。
- 应用的 CSP 为 `null`，从 Blob 创建 Worker 不受限制。

## 登录流程
在 `omnyssh-core/src/ssh/session.rs` 的 `authenticate` 开头、试 SSH agent 之前，加一步"生成的密钥"。只有同时满足下面三个条件才执行：
1. 总开关已打开；
2. 主机设置了 `key_generator`，且该生成器存在；
3. 有前端注册了生成服务。GUI 启动时注册；TUI 不注册。

执行时：
1. 先查内存缓存。缓存的键由生成器的代码和凭证、主机的 MAC 和 revision、主机的 name/hostname/user/port 组合而成，任何一项变化都会重新生成。只缓存生成成功的私钥，在本次运行期间有效。
2. 缓存里没有，就发 `CoreEvent::KeyRequired { request_id, host_name, generator, mac, revision, address }`，最多等 15 秒（`address` 是主机的 Hostname / IP）。前端用 Worker 运行代码，再调用 `answer_generated_key(request_id, Ok(私钥) | Err(原因))` 回答。
3. 解析私钥，用它做一次公钥认证。
   - 成功：登录完成。
   - 不成功（生成失败、超时、解析失败、服务器不接受）：记下原因，继续执行原来的流程（agent → 密钥文件 → 默认密钥 → 密码），一切照旧。
4. 如果最后整个登录都失败了，在原有的错误信息后面附上 `key generator '<名字>': <原因>`。

这一步对交互连接（终端、SFTP、拖放上传）和后台连接（仪表盘监控、端口转发、快捷片段）一样生效，因为生成不需要人参与。

## 界面
- **设置页**新增"Key generators"一节：
  - 总开关，默认关闭。
  - 打开后显示生成器列表，可以新增、修改、删除。
  - 编辑框：名字、代码和"Credential (JSON)"（都是等宽字体的多行输入框；凭证不是合法 JSON 时不能保存），以及"测试"区：填 MAC、revision、地址，点 Run，显示生成密钥的公钥指纹（`SHA256:…`）或错误信息。私钥本身不显示。如果返回值里的 `fingerprint` 和算出来的不一致，额外提示一行。
    - 指纹由后端命令 `key_fingerprint(private_key)` 计算：用登录时同一套解析逻辑解析私钥，返回指纹或解析错误。这样"测试通过"就等于"登录时能用"。私钥只在这次调用的内存里出现。
  - 删除一个还有主机在用的生成器时，列出这些主机并确认；确认后，同时清掉这些主机的 `key_generator`。
  - 改名时，同步更新引用它的主机。
- **主机表单**：总开关打开时才显示：
  - "Key generator"下拉框，选项是"None"加各个生成器，默认 None。
  - "MAC"（必填）和"Revision"（选填，正整数）两个输入框，选了生成器才显示。
- **主机卡片**：设置了生成器的主机显示一个小标签 `gen`，悬停显示生成器名。总开关关闭时不显示。
- 界面文字都是英文。

## 错误处理
| 情况 | 处理 |
|---|---|
| 总开关关闭 | 不发请求，与现在完全一样 |
| 主机引用的生成器不存在 | 跳过 |
| 没有前端注册（TUI），或前端 15 秒内没回答 | 跳过 |
| JS 抛出异常、超过 10 秒、返回的不是私钥 | 跳过，记下原因 |
| 私钥解析失败（格式不对、带了密码） | 跳过，记下原因 |
| 服务器不接受这把密钥 | 继续原来的流程 |
| 整个登录失败 | 原有错误信息后附上生成器的原因 |

"测试"按钮的错误直接显示在编辑框里，不影响任何连接。

## 测试
- **Rust core**，用进程内的测试 SSH 服务器做端到端测试，服务器只接受某把指定的公钥：
  - 生成的密钥能登录。
  - 生成失败、或服务器不接受生成的密钥时，会继续用密码登录成功。
  - 总开关关闭时，不发 `KeyRequired`。
  - 第二次连接直接用缓存，不再发请求；改了 MAC 或 revision 会重新请求。
  - 没有注册生成服务时，直接跳过，不等待。
- **GUI 后端**：`KeyRequired` 到前端事件的映射；`answer_generated_key` 的往返；15 秒超时后跳过。生成器配置的读写和校验（名字唯一、改名同步、删除清引用）。`key_fingerprint` 对 OpenSSH 和 PEM 格式的 RSA、ECDSA、Ed25519 私钥给出正确指纹，对带密码的私钥和非私钥文本返回错误。
- **前端单元测试**：Worker 运行器按 `({ credential, mac, revision? }, address)` 调用 `generateKey`，没填 revision 时不带这个键；能取出 `privateKey`、能捕获异常、超时会被终止、能识别缺少 `privateKey` 或不是私钥的返回值；使用 `crypto.subtle` 的代码能正常运行。
- **e2e**：
  - 总开关关闭时，主机表单和卡片上没有相关内容。
  - 打开后新增生成器（含凭证 JSON），"测试"显示公钥指纹；凭证不是合法 JSON 时不能保存。
  - 主机选择生成器并保存；复制主机时字段一起带过去。
  - 后端发来 `key-required` 时，前端会自动运行代码并回答。
  - 删除被引用的生成器会提示，确认后清掉引用。
- **TUI**：编辑主机后这两个字段原样保留；连接这类主机时照常登录。

## 不做
- 不把生成的私钥写入磁盘，也不显示私钥。
- 不支持带密码的私钥。
- 不支持生成密码（只生成私钥）。
- TUI 不运行生成器。
- 不检查 MAC 的格式，原样交给代码。
