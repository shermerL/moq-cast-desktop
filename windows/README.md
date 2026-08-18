# MoQCast Windows

本目录是 `moq-cast-desktop` 单一桌面仓库的 Windows 平台实现，与同级 `linux/` 保持平台代码隔离。Windows 产品包含完整 MoQCast Windows 桌面端，以及只向本机浏览器提供设备发现结果的 Browser LAN Bridge；本目录不包含 Linux 或 Android 平台代码。

当前 Windows 桌面端已经包含 W1/W2 发现与安全会话基础、deterministic mesh、共享 Origin、单路屏幕发布，以及远端 screen catalog 订阅和 Live Player。Nearby 根据上游 `should_dial` 自动直连，不提供手动 Connect/Disconnect；短暂 mDNS Lost 不会拆除健康 QUIC session。屏幕发布固定使用 `moqcast.screen/<local-peer-id>`，观看与发布互斥，停止媒体不会拆除 mesh。

界面内置 `assets/fonts/NotoSansSC-Regular.otf` 作为 proportional 与 monospace 的最低优先级简体中文 fallback，不替换默认拉丁字体。字体采用 SIL Open Font License，许可证见 `assets/fonts/LICENSE-NOTO`。

Windows 屏幕发布使用 Desktop Duplication 与 H.264，优先 Media Foundation 硬件编码并保留 OpenH264 回退。系统音频只采集默认 render endpoint 的 WASAPI loopback，不申请或采集麦克风；PCM 被规范化为 48 kHz stereo Opus，并与视频共享 publication Clock。音频采集或编码失败只更新独立音频状态，不结束视频发布。首版安全支持 mono/stereo mix format，多声道输出设备会明确标为不支持而不会按未知 channel mask 静默下混。

远端播放会从 Hang catalog 选择同一 broadcast 中受支持的 Opus 或 PCM rendition，复用 pinned `moq-audio` 的 decoder 与 CPAL/WASAPI 默认输出设备。音频订阅和设备生命周期运行在独立任务中，因此设备打开、track 结束或输出失败不会阻塞视频首帧，也不会结束视频播放。当前只提供 bounded jitter/resample 播放，不宣称已经完成严格的音画时钟同步。

Desktop Duplication 尚未合成硬件 overlay 鼠标指针。根因与修复边界位于上游 `moq-video::capture::desktopduplication`，桌面端不维护第二套 capture workaround；上游完成 cursor shape 缓存与合成后再更新 pinned revision。

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

即使 `RUST_LOG` 请求 debug/trace，应用也会把 `moq_native` 与 `mdns_sd` 限制到 warn，防止底层 DNS-SD 调试日志打印 TXT fingerprint、nonce 或 credential 派生材料。

## 验证

```powershell
cargo fmt --all --check
cargo check --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

macOS 上的纯逻辑测试不会编译或运行 WASAPI、Desktop Duplication、Media Foundation/D3D11/DXVA 或 Windows 音频输出。Windows CI 只能证明 Windows runner 上能够编译和运行自动测试。真实 Found/Updated/Lost、多网卡、IPv4/IPv6、TLS/QUIC、防火墙、GPU codec、系统音频采集、默认输出设备、设备切换、音画表现与 shutdown 行为仍需 Windows 真机和 Android/Linux peer 联调。
