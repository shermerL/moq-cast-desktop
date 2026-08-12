# MoQCast Windows

本目录是 `moq-cast-desktop` 单一桌面仓库的 Windows 平台实现，与同级 `linux/` 保持平台代码隔离。Windows 产品包含完整 MoQCast Windows 桌面端，以及只向本机浏览器提供设备发现结果的 Browser LAN Bridge；本目录不包含 Linux 或 Android 平台代码。

W1 discovery CLI 已完成。W2 增加私有 session foundation：进程拥有真实 MoQ/QUIC listener，发现记录使用 listener 的实际端口和证书 fingerprint，并按 upstream `should_dial` 建立 fingerprint-pinned session。typed transport state、per-peer generation、Disconnect 和 shutdown 由 session foundation 管理；当前仍不传输媒体。

## W2 session foundation

```powershell
cargo run -- --bind "[::]:0"
```

listener 默认绑定 `[::]:0`，启动后把实际端口和自动生成证书的 SHA-256 fingerprint 交给 mDNS。发现、连接与媒体仍是分开的层；看到 peer 不等于 TLS、credential 或 MoQ session 已成功。

若局域网成员使用共享 secret，只接受 secret 文件，避免把 secret 直接放进进程参数：

```powershell
cargo run -- --bind "[::]:0" --secret-file C:\path\to\lan-secret.txt
```

文件内容必须是 32 字节 secret 的 64 位十六进制编码。CLI 日志不会输出 secret、peer credential 或完整 TLS fingerprint。

即使 `RUST_LOG` 请求 debug/trace，应用也会把 `moq_native` 与 `mdns_sd` 限制到 warn，防止底层 DNS-SD 调试日志打印 TXT fingerprint、nonce 或 credential 派生材料。

## 验证

```powershell
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

Windows CI 只能证明 Windows runner 上能够编译和运行自动测试。真实 Found/Updated/Lost、多网卡、IPv4/IPv6、TLS/QUIC、防火墙和 shutdown 行为仍需 Windows 真机与 Android/Linux peer 联调。
