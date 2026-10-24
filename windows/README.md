# MoQCast Windows

本目录是 `moq-cast-desktop` 单一桌面仓库的完整 Windows 桌面端实现，与同级 `linux/` 保持平台代码隔离。独立的轻量 discovery tray 与 loopback presence bridge 位于 [`windows-lite/`](../windows-lite/README.md)。本目录不包含 Linux 或 Android 平台代码。

当前 Windows 桌面端已经包含 W1/W2 发现与安全会话基础、deterministic mesh、共享 Origin、单路屏幕发布，以及远端 screen catalog 订阅和 Live Player。Nearby 根据上游 `should_dial` 自动直连，不提供手动 Connect/Disconnect；短暂 mDNS Lost 不会拆除健康 QUIC session。屏幕发布固定使用 `moqcast.screen/<local-peer-id>`，观看与发布互斥，停止媒体不会拆除 mesh。

界面由 shared-ui 内嵌 Noto Sans SC，统一中文与英文正文，默认字重400；等宽日志保留等宽字体并使用中文后备。字体与 SIL Open Font License 见 `../shared/ui/assets/fonts/`。

屏幕发布使用上游 moq-video 的 Windows Graphics Capture（WGC）与 H.264。WGC 要求 Windows 10 2004（build 19041）或更新系统及受支持的图形设备；旧系统会明确拒绝采集，不回退到 Desktop Duplication/GDI。默认兼容模式保持显示器原生尺寸，最长边不超过 1920，并由 `Auto` 优先选择 Media Foundation、在硬件 encoder 打开失败时尝试 OpenH264；该兼容规则不新增宽高比限制。可选的原生 QHD 模式当前只接受横屏 2560x1440，并强制使用硬件 H.264；应用在开始共享时验证显示器尺寸和 encoder，失败会明确结束本次发布，绝不降级到 OpenH264。当前应用策略仍不提供竖屏 QHD、4K 或缩放模式；更换采集后端不自动扩大已验证的编码策略。

系统音频只采集默认 render endpoint 的 WASAPI loopback，不申请或采集麦克风；PCM 被规范化为 48 kHz stereo Opus，并与视频共享 publication Clock。音频采集或编码失败只更新独立音频状态，不结束视频发布。首版安全支持 mono/stereo mix format，多声道输出设备会明确标为不支持而不会按未知 channel mask 静默下混。

远端播放会从 Hang catalog 选择同一 broadcast 中受支持的 Opus 或 PCM rendition，复用 pinned `moq-audio` 的 decoder 与 CPAL/WASAPI 默认输出设备。音频订阅和设备生命周期运行在独立任务中，因此设备打开、track 结束或输出失败不会阻塞视频首帧，也不会结束视频播放。当前只提供 bounded jitter/resample 播放，不宣称已经完成严格的音画时钟同步。

WGC 后端已合入 moq-dev main，鼠标由 WGC 的 cursor 设置控制。无边框依赖 Windows 版本及系统授权，无法关闭时保留系统捕获边框。应用在共享开始前列出当前可捕获的显示器和可见窗口，并按来源类型和 opaque id 重新核对选择；共享中不能切换来源。窗口标题或尺寸在刷新时变化不会被误判为新窗口，关闭窗口会结束发布，已起播的窗口最小化时暂停新画面，恢复后由后端继续采集；起播前已最小化的窗口需要先还原。窗口共享仍采集默认 render endpoint 的完整系统声音，不提供窗口或进程专属音频。

当前编码尺寸策略只在开始共享时按来源的当前原生尺寸验证。固定的 `moq-video` 会在窗口尺寸变化后于内部重开 WGC 和编码器，但没有向应用暴露重新校验动态尺寸的回调；因此本版本不承诺 resize 后仍持续满足 1920/QHD 上限，也不提供自动缩放。如需强制动态尺寸上限，需要另行评估下层尺寸约束或缩放支持，不能仅靠选源 UI 保证。

依赖固定到 moq-dev main `f8215bc47199b48512d805ae9fc710cc6586de51`，manifest、Cargo.lock 与构建来源保持一致。采集接入上游 `encode::Capture`，实时播放从最新缓存组开始，保留音视频 80ms、纯视频 0ms 的 freshness budget。新基线仍需真机验证。已确认单显示器 Windows 11 25H2 整屏采集正常且鼠标可见；静止画面持续交付、首帧等待竞态、颜色、窗口关闭/最小化/恢复/resize、重复共享及混合 GPU 场景仍需相应真机验证。升级采集后端后请重新确认目标来源，旧 display/window id 不保证对应同一来源。

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

普通日志将 `moq_tokio` 与 `mdns_sd` 限制到 warn。设置中开启详细诊断后，允许 `moq_tokio` 的连接、客户端、服务端、QUIC 与 mDNS 适配层输出 INFO/DEBUG，保留原始错误及端点信息；`mdns_sd` 仍不启用底层 TXT 调试输出。

## 验证

```powershell
cargo fmt --all --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

macOS 上的纯逻辑测试不会编译或运行 WASAPI、WGC、Media Foundation/D3D11/DXVA 或 Windows 音频输出。Windows CI 只能证明 Windows runner 上能够编译和运行自动测试。真实 Found/Updated/Lost、多网卡、IPv4/IPv6、TLS/QUIC、防火墙、GPU codec、系统音频采集、默认输出设备、设备切换、音画表现与 shutdown 行为仍需 Windows 真机和 Android/Linux peer 联调。

## Nearby 多地址连接

普通 LAN 设备的同一份广告地址作为一个连接目标，复用上游 QUIC 交错拨号；某个地址不响应时，后续候选不必等待它超时。地址保留 IPv6 scope，沿用证书指纹与凭据校验、原有连接预算和取消机制。只有握手赢家进入业务会话；显式 node URL 保持原有处理。单独的 mDNS Lost 仍不会拆除健康连接。

三端消费者锁文件使用 `mdns-sd 0.21.4`，包括冲突改名后的 goodbye 修复。这不能代替跨重启身份管理，也不保证所有网络环境都能连接。

## UI v2 开发候选

UI v2 候选复用 shared-ui 的主题、品牌、页头、设备行、弹层与播放器。观看从附近设备进入，返回列表不停止播放；WGC 来源页保持屏幕/窗口切换、刷新和启动前校验，开始/停止区固定。设置保留日志导出、语言及跟随系统/浅色/深色。系统音频沿用既有采集行为，不显示无效的可编辑开关。

Mac 主机上的 UI/状态测试不覆盖 Windows WGC、WASAPI、GPU 或原生窗口；目标编译、125%/150%/200% DPI、全屏和媒体回归仍须分别验收。

## Nearby 状态与操作权限

设备发现、精确关联连接和可观看内容由[共享状态策略](../shared/lan-state/README.md)组合显示。单独 mDNS Lost 不会隐藏有效连接或广播；观看根据有效路由准入，不用无关设备的全局入站数量判断。本机 LAN 就绪后可开始共享并等待观看者，发布与观看仍互斥。关闭过程中禁止新的共享和观看。

匿名入站会话目前不能可靠映射到某个 mDNS 设备，不能据此宣称该设备已连接。当前关闭仅有应用 owner 的可观察清理；上游 mDNS Drop 仍是 best-effort 请求，不是 daemon/socket 全部释放的证明。

关闭 LAN 时会等待出站连接循环和本应用拥有的清理任务退出；主动取消与任务异常分别处理。一个任务清理失败不会跳过其余资源，详细错误记录到日志，界面保留关闭异常。mDNS 的底层 daemon 退出仍不可等待，不能把应用任务结束等同于远端立即离线。

启动期间也会处理关闭命令：尚未开始的构造不再执行；已经开始的构造保持所有权，返回后清理结果，期间保持“关闭中”。这避免主动丢弃旧版 mDNS 构造，但不修复其内部错误路径的清理缺口，也不新增 daemon 完全退出的保证。

## LAN 地址刷新与诊断

同一设备的拨号尚未完成时，发现新增地址会更新候选连接，沿用本轮原有截止时间；重复地址不重新计时，已建立的连接不因补充地址而被替换。凭据变化继续按身份替换处理，旧代次的结果不能覆盖新连接。

联调前在设置中开启详细诊断，再开启附近设备。日志保留设备 ID、发现地址、刷新前后地址、连接代次、剩余预算、逐地址失败原文、握手阶段和连接关闭原因；正常模式不放开底层 DEBUG。入站编号只表示本机接纳的会话，不推断它属于某个 mDNS 设备。导出三端日志并提供操作时间，可对齐事件顺序；日志不能替代抓包或保证复现当时的网络环境。
