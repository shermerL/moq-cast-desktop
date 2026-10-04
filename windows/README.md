# MoQCast Windows

本目录是 `moq-cast-desktop` 单一桌面仓库的完整 Windows 桌面端实现，与同级 `linux/` 保持平台代码隔离。独立的轻量 discovery tray 与 loopback presence bridge 位于 [`windows-lite/`](../windows-lite/README.md)。本目录不包含 Linux 或 Android 平台代码。

当前 Windows 桌面端已经包含 W1/W2 发现与安全会话基础、deterministic mesh、共享 Origin、单路屏幕发布，以及远端 screen catalog 订阅和 Live Player。Nearby 根据上游 `should_dial` 自动直连，不提供手动 Connect/Disconnect；短暂 mDNS Lost 不会拆除健康 QUIC session。屏幕发布固定使用 `moqcast.screen/<local-peer-id>`，观看与发布互斥，停止媒体不会拆除 mesh。

界面内置 `assets/fonts/NotoSansSC-Regular.otf` 作为 proportional 与 monospace 的最低优先级简体中文 fallback，不替换默认拉丁字体。字体采用 SIL Open Font License，许可证见 `assets/fonts/LICENSE-NOTO`。

屏幕发布使用上游 moq-video 的 Windows Graphics Capture（WGC）与 H.264。WGC 要求 Windows 10 2004（build 19041）或更新系统及受支持的图形设备；旧系统会明确拒绝采集，不回退到 Desktop Duplication/GDI。默认兼容模式保持显示器原生尺寸，最长边不超过 1920，并由 `Auto` 优先选择 Media Foundation、在硬件 encoder 打开失败时尝试 OpenH264；该兼容规则不新增宽高比限制。可选的原生 QHD 模式当前只接受横屏 2560x1440，并强制使用硬件 H.264；应用在开始共享时验证显示器尺寸和 encoder，失败会明确结束本次发布，绝不降级到 OpenH264。当前应用策略仍不提供竖屏 QHD、4K 或缩放模式；更换采集后端不自动扩大已验证的编码策略。

系统音频只采集默认 render endpoint 的 WASAPI loopback，不申请或采集麦克风；PCM 被规范化为 48 kHz stereo Opus，并与视频共享 publication Clock。音频采集或编码失败只更新独立音频状态，不结束视频发布。首版安全支持 mono/stereo mix format，多声道输出设备会明确标为不支持而不会按未知 channel mask 静默下混。

远端播放会从 Hang catalog 选择同一 broadcast 中受支持的 Opus 或 PCM rendition，复用 pinned `moq-audio` 的 decoder 与 CPAL/WASAPI 默认输出设备。音频订阅和设备生命周期运行在独立任务中，因此设备打开、track 结束或输出失败不会阻塞视频首帧，也不会结束视频播放。当前只提供 bounded jitter/resample 播放，不宣称已经完成严格的音画时钟同步。

WGC 后端已合入 moq-dev main，鼠标由 WGC 的 cursor 设置控制。无边框依赖 Windows 版本及系统授权，无法关闭时保留系统捕获边框。应用在共享开始前列出当前可捕获的显示器和可见窗口，并按来源类型和 opaque id 重新核对选择；共享中不能切换来源。窗口标题或尺寸在刷新时变化不会被误判为新窗口，关闭窗口会结束发布，已起播的窗口最小化时暂停新画面，恢复后由后端继续采集；起播前已最小化的窗口需要先还原。窗口共享仍采集默认 render endpoint 的完整系统声音，不提供窗口或进程专属音频。

当前编码尺寸策略只在开始共享时按来源的当前原生尺寸验证。固定的 `moq-video` 会在窗口尺寸变化后于内部重开 WGC 和编码器，但没有向应用暴露重新校验动态尺寸的回调；因此本版本不承诺 resize 后仍持续满足 1920/QHD 上限，也不提供自动缩放。如需强制动态尺寸上限，需要另行评估下层尺寸约束或缩放支持，不能仅靠选源 UI 保证。

依赖固定到 moq-dev main 中的完整提交，manifest、Cargo.lock 与构建来源保持一致。已确认单显示器 Windows 11 25H2 整屏采集正常且鼠标可见；静止画面持续交付、首帧等待竞态、颜色、窗口关闭/最小化/恢复/resize、重复共享及混合 GPU 场景仍需相应真机验证。升级采集后端后请重新确认目标来源，旧 display/window id 不保证对应同一来源。

## 启动桌面端

```powershell
cargo run -- --bind "[::]:0"
```

listener 默认绑定 `[::]:0`，后台 runtime owner 启动后把实际端口和自动生成证书的 SHA-256 fingerprint 交给 mDNS。发现、连接与媒体仍是分开的 typed state；看到 peer 不等于 TLS、credential 或 MoQ session 已成功。

若局域网成员使用共享 secret，只接受 secret 文件，避免把 secret 直接放进进程参数：

```powershell
cargo run -- --bind "[::]:0" --secret-file C:\path\to\lan-secret.txt
```

文件内容必须是 32 字节 secret 的 64 位十六进制编码。应用日志和 UI snapshot 不会输出 secret、peer credential 或完整 TLS fingerprint。

即使 `RUST_LOG` 请求 debug/trace，应用也会把 `moq_tokio` 与 `mdns_sd` 限制到 warn，防止底层 DNS-SD 调试日志打印 TXT fingerprint、nonce 或 credential 派生材料。

## 验证

```powershell
cargo fmt --all --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

macOS 上的纯逻辑测试不会编译或运行 WASAPI、WGC、Media Foundation/D3D11/DXVA 或 Windows 音频输出。Windows CI 只能证明 Windows runner 上能够编译和运行自动测试。真实 Found/Updated/Lost、多网卡、IPv4/IPv6、TLS/QUIC、防火墙、GPU codec、系统音频采集、默认输出设备、设备切换、音画表现与 shutdown 行为仍需 Windows 真机和 Android/Linux peer 联调。
