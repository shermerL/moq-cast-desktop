# MoQCast Windows

本目录是 `moq-cast-desktop` 单一桌面仓库的 Windows 平台实现，与同级 `linux/` 保持平台代码隔离。Windows 产品包含完整 MoQCast Windows 桌面端，以及只向本机浏览器提供设备发现结果的 Browser LAN Bridge；本目录不包含 Linux 或 Android 平台代码。

当前正在推进 W1 discovery CLI。CLI 复用固定 moq-dev revision 的 `_moq._udp.local.` 记录、身份、credential 和 `should_dial` 语义，只验证发现层，不建立 MoQ session，也不传输媒体。

## W1 discovery CLI

```powershell
cargo run -- --port 4443 --fingerprint <SHA256_HEX>
```

`--port` 必须对应同一进程体系中真实 MoQ listener 的端口。W1 spike 本身不打开 listener，因此不能把看到 peer 解释为连接成功。

若局域网成员使用共享 secret，只接受 secret 文件，避免把 secret 直接放进进程参数：

```powershell
cargo run -- --port 4443 --fingerprint <SHA256_HEX> --secret-file C:\path\to\lan-secret.txt
```

文件内容必须是 32 字节 secret 的 64 位十六进制编码。CLI 日志不会输出 secret、peer credential 或完整 TLS fingerprint。

即使 `RUST_LOG` 请求 debug/trace，应用也会把 `moq_native` 与 `mdns_sd` 限制到 warn，防止底层 DNS-SD 调试日志打印 TXT fingerprint、nonce 或 credential 派生材料。

## 验证

```powershell
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

Windows CI 只能证明 Windows runner 上能够编译和运行自动测试。真实 Found/Updated/Lost、多网卡、IPv4/IPv6 和防火墙行为仍需 Windows 真机与 Android/Linux peer 联调。
