# MoQCast macOS

## UI v2

桌面端已接入 `moqcast_ui::v2`，采用本机卡片、附近设备列表及固定页脚的新布局。设备名称/信息按钮打开详情，观看按钮进入播放器；设置和详情使用弹层。共享设置保留系统选择器，开始/停止操作固定在可见区域。浅色、深色、跟随系统可切换并持久保存。

返回附近设备不停止播放；媒体互斥、Nearby 关闭确认、采集权限与 runtime 状态契约沿用现有实现。播放器保持 LIVE 及原红色，画面按比例使用独立黑色舞台，工具栏跟随主题，等待只保留转圈。字体随二进制内嵌，默认字重明确为400。三端均已接入新 UI 并集成 main；已知问题与发行状态见仓库更新日志。

从 `mac/` 目录使用 `cargo +1.95.0 run --release --locked` 运行，以读取本目录的 macOS 14.2 部署配置。组件契约与目录入口见 `../shared/ui/README.md`。

更新时间：2026-10-19 Asia/Shanghai

`mac/` 是 MoQCast macOS 桌面端的唯一产品代码目录。当前实现 Nearby direct-only session、ScreenCaptureKit 屏幕/窗口共享与原生音视频播放。

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

三端消费者锁文件使用 `mdns-sd 0.21.4`，包括冲突改名后的 goodbye 修复。这不能代替跨重启身份管理，也不保证所有网络环境都能连接。

## Nearby 状态与操作权限

设备发现、精确关联连接和可观看内容由[共享状态策略](../shared/lan-state/README.md)组合显示。单独 mDNS Lost 不会隐藏有效连接或广播；观看根据有效路由准入，不用无关设备的全局入站数量判断。本机 LAN 就绪后可开始共享并等待观看者，发布与观看仍互斥。关闭过程中禁止新的共享和观看。

匿名入站会话目前不能可靠映射到某个 mDNS 设备，不能据此宣称该设备已连接。当前关闭仅有应用 owner 的可观察清理；上游 mDNS Drop 仍是 best-effort 请求，不是 daemon/socket 全部释放的证明。

关闭 LAN 时会等待出站连接循环和本应用拥有的清理任务退出；主动取消与任务异常分别处理。一个任务清理失败不会跳过其余资源，详细错误记录到日志，界面保留关闭异常。mDNS 的底层 daemon 退出仍不可等待，不能把应用任务结束等同于远端立即离线。

固定依赖仍有 mDNS 创建完成前取消的清理缺口；启动中关闭、重启或休眠涉及这一路径。现有应用任务退出不构成底层 daemon 已退出的证明，此边界仍需上游修复接入后验证。

## LAN 地址刷新与诊断

同一设备的拨号尚未完成时，发现新增地址会更新候选连接，沿用本轮原有截止时间；重复地址不重新计时，已建立的连接不因补充地址而被替换。凭据变化继续按身份替换处理，旧代次的结果不能覆盖新连接。

联调前在设置中开启详细诊断，再开启附近设备。日志保留设备 ID、发现地址、刷新前后地址、连接代次、剩余预算、逐地址失败原文、握手阶段和连接关闭原因；正常模式不放开底层 DEBUG。入站编号只表示本机接纳的会话，不推断它属于某个 mDNS 设备。导出三端日志并提供操作时间，可对齐事件顺序；日志不能替代抓包或保证复现当时的网络环境。
