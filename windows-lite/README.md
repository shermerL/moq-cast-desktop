# MoQTCast Lite for Windows

MoQTCast Lite 是一个独立的轻量 Windows notification-area 应用。它只浏览 `_moq._udp.local.`，显示经过清洗的在线设备数量，并通过进程期授权的 loopback API 把 presence 交给 <https://moqtcast.com/connect>。

MoQTCast Lite is a separate lightweight Windows notification-area application. It only browses `_moq._udp.local.`, shows a sanitized online-device count, and hands presence to <https://moqtcast.com/connect> through a process-authorized loopback API.

## 产品边界 / Product boundary

- 只查询 DNS-SD，不注册或广播服务，因此不会冒充 MoQ 媒体节点。
- 发现结果只表示设备在线，不证明媒体端点可达。
- 应用不采集、编码、解码或播放媒体，不拨号、不创建 mesh，也不代理 MoQ 或媒体流量。
- Registry 私下保留当前 resolved 服务的 instance、port、TXT `fp`/`n` 与最多 8 个过滤后的 IPv4；这些字段及原始服务名不进入 presence、托盘 UI、应用日志或 `Debug` 输出。TXT `node`、IPv6 和未知 TXT 在首版被忽略。
- loopback 只绑定动态 `127.0.0.1` 端口。每次启动生成新的 256-bit token，不扫描端口，也不持久化 token。
- 托盘的 Open MoQTCast 只使用 `https://moqtcast.com/connect#lite=<base64url(JSON UTF-8, no padding)>` 交接启动信息。解码后的 JSON 恰好是 `{"version":1,"endpoint":"http://127.0.0.1:<动态端口>","token":"<43-char base64url>"}`，不接受或生成旧 fragment 格式。fragment 不进入服务器请求、查询参数或应用日志。
- `GET /v1/presence` 只返回 schema、revision、lifecycle、进程期 opaque device id、清洗后 display name、online 和 `watchable`。首版使用 1 至 2 秒 polling，不使用 SSE 或 WebSocket。
- API 精确校验实际 Host、`https://moqtcast.com` Origin、Bearer token、method 与 path，并设置 no-store、nosniff、CORS allowlist、请求/响应尺寸和速率上限。浏览器预检 `OPTIONS` 不携带 token，但必须声明 `GET` 与 `Authorization`。
- 只有无 TXT `a`、16 位小写 hex instance、非零 port、64 位 hex `fp`、32 位 hex `n` 且至少有一个可用 IPv4 的设备才标为 `watchable=true`。用户选择该设备后，`GET /v1/presence/<opaque-id>/watch-descriptor` 才返回进程内即时生成的 experimental open-cluster descriptor；不存在、离线或不可看的设备统一返回 404。
- Lite 仍不注册 mDNS、不拨号、不创建 mesh，也不处理、代理或转发媒体。浏览器通过 WebTransport 直接连接完整 Desktop，Lite 不进入媒体路径。

## Dev PoC 权限说明 / Security notice

当前 `experimental-open-cluster` descriptor 仅用于已明确授权的 Windows、Linux 和 Android 网页播放开发验证。受保护的 loopback API 会把 endpoint、证书指纹和临时 open-cluster credential 交给 `https://moqtcast.com` 页内内存；这些值不得进入 query、fragment、日志、持久化存储、analytics 或 Cloudflare 服务端。

当前 `/.cluster/<n>` 会话具备双向 MoQ 协议权限，不是 watch-only capability。`watchable=true` 只表示记录满足本 PoC 的严格形状，不表示生产级最小权限已完成。正式 watch-only 服务与 descriptor 契约后续单独实现。

## 构建 / Build

项目使用 Rust `1.95.0`。Windows Release 使用 GUI subsystem，不弹出 console。

```powershell
cargo fmt --all --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

Windows CI 只负责 Windows 编译、自动测试、Clippy、release build 和 PE GUI subsystem 断言。托盘消息循环、菜单、mDNS、防火墙和浏览器打开仍需 Windows 真机验收。其他平台的单元测试只覆盖 registry、去重、过期和 redaction 逻辑。

本地或 CI 单元测试还覆盖 token/fragment、Host/Origin/auth/method/path、CORS 预检、容量、请求/响应边界、限流、shutdown 和旧 token 失效。它们不证明 Chrome 的 Local Network Access、mixed content 策略或 Windows notification-area 行为。

## Dependencies and licenses

- `mdns-sd` 0.21.0: MIT OR Apache-2.0
- `base64` 0.23.1: MIT OR Apache-2.0
- `getrandom` 0.4.3: MIT OR Apache-2.0
- `httparse` 1.10.1: MIT OR Apache-2.0
- `serde_json` 1.0.151: MIT OR Apache-2.0
- `tray-icon` 0.24.2: MIT OR Apache-2.0, Windows target only
- `windows-sys` 0.61.2: MIT OR Apache-2.0, Windows target only

本项目自身采用仓库根目录的 MIT OR Apache-2.0 双许可证。
