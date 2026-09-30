# Desktop 构建来源

更新：2026-09-29。

Windows、macOS、Linux 的 `build.rs` 调用 `generate()`，读取本平台 Cargo.toml、Cargo.lock 和源码状态，在 OUT_DIR 生成 Rust 常量与 `moqcast-build-info.txt`。应用诊断使用这些常量，打包复用同一份文本，不再手写另一份 MoQ SHA。

- 记录应用版本、构建身份、源码提交/状态、MoQ Git 基线和 Cargo 的实际 TARGET。
- 同一应用的 MoQ Git 依赖必须使用同一仓库与完整固定 SHA；锁文件的请求和解析 SHA 必须匹配。独立发布的 registry crate 仍由 Cargo 的 `--locked` 解析约束。
- Linux 额外核对 moq-video 的 VENDORED.md、其 Git 依赖、path lock 条目，以及 libspa 的 manifest/来源记录/lock。记录 vendor 身份和 manifest 声明的 features，源码提交对应本地补丁。这里不是完整依赖清单，也不宣称 vendor 与上游原版逐字相同。
- Git checkout 自动记录 HEAD，未提交修改标记为 dirty。没有 `.git` 的归档为 unknown；显式提供来源提交的归档为 provided，不能视作 Git 已核验的 clean。
- 生成器监听源码和 Git 元数据变化；忽略的文件不属于源码身份。错误或不一致输入使构建失败。

可选环境变量：

| 变量 | 用途 |
| --- | --- |
| MOQCAST_BUILD_IDENTITY | CI 的包变体，缺省 local |
| MOQCAST_SOURCE_COMMIT | 完整源码 SHA；存在 Git 时必须等于 HEAD |
| MOQCAST_PROVENANCE_OUTPUT | 将同一文本另存到包流程路径，通常放在已忽略的 target/ 下；相对路径以平台目录为基准 |

macOS 单架构记录保留真实 target；Universal 2 打包确认两份记录除 target 外一致后派生包级 target。Linux AppDir 在生成记录后追加系统库、发行版和构建日期。Windows 内部校验生成记录，公开产物及 minimal 诊断导出范围保持原样。

验证：`cargo test --locked --manifest-path shared/build-provenance/Cargo.toml`。Linux 打包合同由 `linux/scripts/check-build-provenance.sh` 验证。三端 workflow 均覆盖此目录；各平台完整编译与打包仍由对应 CI 执行。
