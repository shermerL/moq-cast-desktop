# LAN state policy

三端共用的纯状态投影与媒体准入策略。没有网络、异步执行器、定时器或第三方依赖，不负责鉴权或资源释放。

- `Peer` 分开记录发现、精确关联连接和可用内容；未知入站连接不填入某个设备。
- `visible/status` 保留有效连接或广播，不把 mDNS Lost 当成传输断开。
- `Access` 从平台 LAN owner 和媒体占用映射。关闭中禁止新媒体；观看依赖有效目录路由，不依赖全局连接计数。
- 发布可以等待观看者，三端不再用当前连接数量决定能否开始本机共享。
- 主状态和中英文标签共用，平台仍负责视觉组件、详细错误和权限提示。

本库不是完整 LAN 状态机，不模拟或保证底层关闭。平台自身的 generation、资源所有权和关闭回归仍需保留。当前匿名入站没有可验证的 mDNS 设备绑定；mDNS Drop 也没有可等待的 daemon/socket 退出屏障。

```sh
cargo test --locked --manifest-path shared/lan-state/Cargo.toml
cargo clippy --locked --manifest-path shared/lan-state/Cargo.toml --all-targets -- -D warnings
```

三端现有 CI 均显式执行此库测试，并在修改 shared/lan-state 时触发相应平台检查。仅构建依赖不会运行依赖内部测试，不能省略此步骤。

任务清理错误汇总放在独立的 `shared/task-cleanup.rs` 内部模块，由三端应用测试覆盖。它复用各应用现有 tokio，不给此纯状态库增加依赖；只汇总异常，不决定资源停止顺序。

## 连接估计视图

`bandwidth::Observation` 仅保存一个当前样本与 epoch，不持有网络对象。连接 owner 独占写入，`View` 是 Weak 观察句柄，owner 停止后读取为空；重连清空旧值，旧 epoch 不能覆盖当前值，未知与零值分开。UI 快照比较句柄身份，不把数值变化当作设备生命周期事件。网络采样保留在三端复用的 `shared/connection-bandwidth.rs` 私有模块中，不在此库增加运行时或定时器。
