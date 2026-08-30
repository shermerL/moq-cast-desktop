# MoQCast macOS

更新时间：2026-08-30 CST

`mac/` 是 MoQCast macOS 桌面端的唯一产品代码目录。当前分支实现 M0/M1 基础，不代表 Nearby、观看或发布媒体已经可用。

## 当前范围

- Rust 1.95.0、eframe/egui 与独立 Tokio runtime owner。
- 容量为 32 的 bounded runtime command channel，以及只发布最新状态的 watch channel。
- discovery、session、capture 与 decoder 的独立 typed lifecycle 和 generation 边界。
- 固定 `_moq._udp.local.`、`/.cluster/<credential>` 与 `moqcast.screen/<peer-id>` 契约。
- 固定 moq-dev revision `81d39f7bf04c82aae324a9ee4251b7f8aa08fb53`，`foundation` feature 预留当前锁定的 network/media 依赖。
- 结构化 stderr diagnostics、运行时 typed snapshot 与构建来源显示。
- macOS 14.2 deployment target、bundle ID `dev.moq.moqcast.macos` 和 ad hoc 签名 Universal 2 `.app` 打包。

尚未实现：Bonjour、listener、QUIC/MoQ session、远端 H.264、ScreenCaptureKit、VideoToolbox 媒体管线、Opus 播放、持久化 DLOG/export、Developer ID 签名、公证与真机验收。

## 本地验证

只运行必要检查，并关注 `mac/target` 占用：

```bash
cd mac
cargo fmt --all --check
cargo test --locked --no-default-features --lib
cargo check --locked
```

`foundation` 会编译 moq-tokio、moq-video、moq-audio、hang 与 moq-mux。双架构和完整 feature 检查交给 macOS CI：

```bash
cargo check --locked --all-features
```

构建成功只证明源码和当前主机工具链，不证明 ScreenCaptureKit 权限、Bonjour、QUIC、VideoToolbox、CoreAudio 或真实设备行为。

M0-M6 开发包使用 ad hoc 签名。测试者可以手动批准 Gatekeeper 提示。Developer ID、公证和 App Store 都不阻塞开发测试。

## 证据边界

- 源码：能够确认契约、状态所有权、generation、依赖 pin 与权限声明。
- 本地构建：能够确认当前主机和 SDK 上的编译/链接。
- CI：能够确认 GitHub macOS runner 上的测试、lint 和双架构 ad hoc 签名打包。
- 真机：必须由明确的应用日志或用户观察确认发现、连接、权限、画面、声音、睡眠/网络切换和生命周期。

发行与凭据边界见 [RELEASE.md](RELEASE.md)。
