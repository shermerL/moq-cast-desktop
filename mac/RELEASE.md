# macOS Release Boundary

更新时间：2026-08-30 CST

首版发行不以 Mac App Store 为门槛。目标路线是 Developer ID Application 签名、Hardened Runtime、Apple 公证、staple 后通过官网和 GitHub Release 分发。

## M0 已冻结

- bundle ID：`dev.moq.moqcast.macos`。
- 最低系统：macOS 14.2。
- `Info.plist` 声明本地网络、`_moq._udp` Bonjour、屏幕捕获与系统音频捕获用途。
- `entitlements.plist` 当前为空。首版不是 App Sandbox，不提前加入 sandbox network entitlement。
- M0-M6 本地和 PR CI 使用 ad hoc 签名 Universal 2 包，允许测试者手动批准 Gatekeeper 提示。
- 开发 CI 不读取 Developer ID 签名或公证 secret。
- 仓库不提交证书、私钥、Apple ID、app-specific password、API key 或 notarytool keychain profile。

## M7 公开发布前待实现

- 使用 Developer ID Application identity 和 `codesign --options runtime --timestamp` 对内层可执行文件与 `.app` 签名。
- 通过 `xcrun notarytool submit --wait` 公证，并用 `xcrun stapler staple` 附加 ticket。
- 验证 `codesign --verify --deep --strict`、`spctl --assess --type execute`、`stapler validate` 与 `lipo -info`。
- release workflow 才允许读取以下 secret：`APPLE_CERTIFICATE_P12`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_NOTARY_KEY_ID`、`APPLE_NOTARY_ISSUER_ID`、`APPLE_NOTARY_KEY_P8`。
- 在签名 release artifact 上验收本地网络提示、Bonjour、屏幕录制授权、系统音频授权、权限拒绝/撤销和重复启动。

## 后续独立里程碑

Mac App Store 另行评估，不阻塞首版。届时重新验证 App Sandbox、`com.apple.security.network.client`、`com.apple.security.network.server`、UDP/mDNS、ScreenCaptureKit、审核中的录屏告知与功能完整性，不直接复用 Developer ID 的空 entitlement 结论。
