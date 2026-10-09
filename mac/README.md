# MoQCast macOS

更新时间：2026-10-09 Asia/Shanghai

`mac/` 是 MoQCast macOS 桌面端的唯一产品代码目录。当前工作树实现 M0-M2 的原生基础与 Nearby direct-only session。观看和发布媒体仍未实现。

## 当前范围

- Rust 1.95.0、eframe/egui 与独立 Tokio runtime owner。
- 容量为 32 的 bounded runtime command channel，以及只发布最新状态的 watch channel。
- `moq-tokio::mdns` Nearby、QUIC listener、fingerprint pin、credential path 与 upstream `should_dial`。
- discovery、session、media、capture 与 decoder 的独立 typed lifecycle 和 generation 边界。
- 固定 `_moq._udp.local.`、`/.cluster/<credential>` 与 `moqcast.screen/<peer-id>` 契约。
- 一个本地 publish Origin 与独立 remote receive Origin。健康 session 不因 mDNS Lost 被拆除。
- Nearby、Screen Share、播放器和 Settings；播放器支持音量、静音与播放信息。
- 固定 moq-dev main revision `f8215bc47199b48512d805ae9fc710cc6586de51`，Cargo manifest、锁文件与构建来源一致。
- 结构化且不含内部身份的普通日志，以及仅供应用内部消费的 typed snapshot。
- macOS 14.2 deployment target、bundle ID `dev.moq.moqcast.macos` 和 ad hoc 签名 Universal 2 `.app` 打包。

ScreenCaptureKit 系统选择器负责屏幕/窗口选择，VideoToolbox 与上游编码接口负责媒体处理；Opus 播放、诊断日志导出已接入。系统声音是否可用按所选来源明确提示。Developer ID 签名、公证尚未接入。

升级后的采集使用上游 `encode::Capture`，音视频共享 catalog 时钟；实时播放从最新缓存组开始，保留音视频 80ms、纯视频 0ms 的 freshness budget。该预算不是端到端延迟保证；新基线仍需真机验证。

## 本地验证

只运行必要检查，并关注 `mac/target` 占用：

```bash
cd mac
cargo fmt --all --check
cargo test --locked --no-default-features --lib
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

`app` 默认包含 `publish`，并通过 `watch` 包含 `network`。当前 macOS CI 运行同一组默认 feature 测试和 Clippy，另行执行 no-default 契约测试；双架构 release build 也使用默认 feature。`foundation` 同样启用 `publish` 媒体依赖。

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

### 构建来源

应用诊断和打包元数据由 [共享生成器](../shared/build-provenance/README.md) 从实际依赖与 Git 状态生成。手工合包时需同时传入各架构的编译记录：

```sh
./scripts/package-app.sh arm64-binary arm64-build-info.txt x86_64-binary x86_64-build-info.txt output-directory
```

各架构编译设置 `MOQCAST_BUILD_IDENTITY=macos-universal2-adhoc` 和 `MOQCAST_PROVENANCE_OUTPUT`；Cargo 的实际 target 自动记录，合包时校验共同来源并派生 Universal 2 元数据。

## Nearby 多地址连接

普通 LAN 设备的同一份广告地址作为一个连接目标，复用上游 QUIC 交错拨号；某个地址不响应时，后续候选不必等待它超时。地址保留 IPv6 scope，沿用证书指纹与凭据校验、原有连接预算和取消机制。只有握手赢家进入业务会话；显式 node URL 保持原有处理。单独的 mDNS Lost 仍不会拆除健康连接。

三端消费者锁文件使用 `mdns-sd 0.21.4`，包括冲突改名后的 goodbye 修复。这不能代替跨重启身份管理，也不保证所有网络环境都能连接；关联 [#39](https://github.com/shermerL/moq-cast-desktop/issues/39)，尚未认定重复设备问题完全解决。
