# MoQCast macOS

更新时间：2026-08-30 CST

`mac/` 是 MoQCast macOS 桌面端的唯一产品代码目录。当前工作树实现 M0-M2 的原生基础与 Nearby direct-only session。观看和发布媒体仍未实现。

## 当前范围

- Rust 1.95.0、eframe/egui 与独立 Tokio runtime owner。
- 容量为 32 的 bounded runtime command channel，以及只发布最新状态的 watch channel。
- `moq-tokio::mdns` Nearby、QUIC listener、fingerprint pin、credential path 与 upstream `should_dial`。
- discovery、session、media、capture 与 decoder 的独立 typed lifecycle 和 generation 边界。
- 固定 `_moq._udp.local.`、`/.cluster/<credential>` 与 `moqcast.screen/<peer-id>` 契约。
- 一个本地 publish Origin 与独立 remote receive Origin。健康 session 不因 mDNS Lost 被拆除。
- 按冻结原型实现的 Nearby、Screen Share 不可用页和 Settings。普通 UI 只显示语言与公开版本。
- 固定 moq-dev revision `81d39f7bf04c82aae324a9ee4251b7f8aa08fb53`，`network` 与 `foundation` feature 分开锁定依赖。
- 结构化且不含内部身份的普通日志，以及仅供应用内部消费的 typed snapshot。
- macOS 14.2 deployment target、bundle ID `dev.moq.moqcast.macos` 和 ad hoc 签名 Universal 2 `.app` 打包。

尚未实现：远端 H.264 Watch、ScreenCaptureKit、VideoToolbox 媒体管线、Opus 播放、持久化 DLOG/export、Developer ID 签名、公证与跨平台真机验收。M2 不显示 Watch、手动 Connect/Disconnect 或未实现的 Screen Share 控件。

## 本地验证

只运行必要检查，并关注 `mac/target` 占用：

```bash
cd mac
cargo fmt --all --check
cargo test --locked --no-default-features --lib
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

`app` 默认包含 `network`。`foundation` 另外编译 moq-video、moq-audio、hang 与 moq-mux。双架构和完整 feature 检查交给 macOS CI：

```bash
cargo check --locked --all-features
```

默认测试包含 loopback listener、credential/fingerprint 拒绝、direct-only Origin 与 generation 状态测试。同机 mDNS smoke 需要本地网络套接字，因此默认忽略并单独运行：

```bash
cargo test --locked --lib network::tests::two_local_services_discover_and_open_one_direct_session -- --ignored --nocapture
```

该 smoke 或构建成功只证明当前 Mac 上的局部机制，不证明 Android、Windows、Linux 真机互操作，也不证明 ScreenCaptureKit、VideoToolbox 或 CoreAudio。

M0-M6 开发包使用 ad hoc 签名。测试者可以手动批准 Gatekeeper 提示。Developer ID、公证和 App Store 都不阻塞开发测试。

## 证据边界

- 源码：能够确认契约、状态所有权、generation、依赖 pin、权限声明与 UI 隐私边界。
- 本地构建：能够确认当前主机和 SDK 上的编译/链接。
- CI：能够确认 GitHub macOS runner 上的测试、lint 和双架构 ad hoc 签名打包。
- 同机 smoke：能够确认两个本地实例曾通过 mDNS 发现并建立一个 direct-only session，不等于跨平台真机。
- 真机：必须由明确的应用日志或用户观察确认 Android、Windows、Linux 的发现和连接，以及后续权限、画面、声音、睡眠/网络切换和生命周期。

发行与凭据边界见 [RELEASE.md](RELEASE.md)。
