# MoQCast shared UI foundation

每次 UI 修改先按 [UI/UX 迭代规范](DESIGN.md) 确认布局与组件契约，并在交付前完成受影响页面的原生检查。测试通过不等于视觉验收通过。

## UI v2：独立迁移入口

`moqcast_ui::v2` 是新的可选视觉入口，不覆盖旧 `Theme` 和组件。Mac、Linux、Windows 均已接入 v2 并集成 main；目标平台原生验收仍分别进行。v2 表示视觉代际，Cargo 包版本独立维护，当前为 0.2.0，见 [更新记录](CHANGELOG.md)。

- `install` 同时安装浅/深主题；平台保存 `ThemePreference`，支持跟随系统。
- `Palette` 统一背景、表面、文本、辅助文本、边框、强调色及悬停色；`dialog` 统一弹层的 24pt 内边距。
- `Metrics` 在窗口完整逻辑内容宽度上判断 920pt 断点，由页面传递，不在嵌套卡片重新计算。
- `ContextCard`、`NearbyRow`、`action`、`icon_button` 负责测量和绘制，不持有连接或媒体对象。设备行分别返回详情、观看事件，由平台按最新 snapshot 判断可执行性。
- 名称可截断并在详情显示全文；动作根据真实字体测量保留完整文字。名称文字不抢占详情点击区域；按钮命中区域至少 40pt。
- 常规页内列表隐藏滚动条但保留滚动和键盘焦点滚入；平台对极端缩放使用可达性滚动兜底。
- 播放器复用既有音量状态与信息数据；`v2::player_rects` 统一完整工作区及全屏布局；`player_toolbar` 随主题显示，黑色视频舞台独立于主题，LIVE 与 `#D92D20` 保留。`canvas_message` 居中呈现失败及恢复动作，等待仅显示转圈。

音量条使用局部样式：4pt 轨道、圆形滑块和 24pt 高的操作区，不继承页面 40pt 按钮的滑块尺寸；静音按钮仍保留 40pt 命中区。浅深主题的悬停、按下与禁用状态复用 egui 交互，音量、静音恢复和音轨时钟语义不变。宽设备行可显示设备 ID，紧凑行优先显示状态，完整 ID 在详情中保留。

| 尺寸（逻辑点） | compact < 920 | wide ≥ 920 |
| --- | ---: | ---: |
| 内容横向边距 | 16 | 24 |
| 页头 / Nearby 页脚 | 56 / 62 | 56 / 62 |
| 本机卡片起点高度 | 112 | 112 |
| 卡片间距 | 12 | 16 |
| 设备行高度 | 76 | 88 |
| 观看动作起点宽度 | 108 | 128 |

原生尺寸以文字测量与可达性为约束，不能把 HTML 像素值当作所有 DPI 下的固定截图承诺。

新目录使用实际组件与明确标记的静态设备，可切浅深主题和 40 行列表：

```sh
cargo +1.95.0 run --locked --example v2_catalog
```

现有 `shared-ui.yml` 的 `--all-targets` 自动编译新目录并运行回归，不新增 CI 工具或依赖。新增回归覆盖紧凑/宽布局、卡片高度、详情与观看点击隔离以及实际绘制文字的左边界。不要只检查 Label 的占位矩形；`Ui::place` 的居中 galley 会让文本仍然错位。迁移其他平台时只复用视觉组件，平台的选源、权限、连接、诊断及生命周期仍由各平台拥有。

`moqcast-ui` contains business-neutral egui 0.36.1 presentation primitives shared by the full Windows, Linux, and macOS desktop applications.

It owns design tokens, semantic typography, interaction-state resolution, centered role-based page shells, caller-data-only components, and the visual catalog. Platform crates continue to own windows, page composition, locale, discovery, media, permissions, diagnostics, and product lifecycle.

Share pages reserve their start/stop action area inside the finite viewport. Application-owned source rows use `v2::source_list` with a bounded height and a floating scrollbar. Refresh and encoding/audio settings stay outside that list; any overflow in the settings uses a separate sibling viewport, never a parent scroll area. System-owned macOS and Wayland pickers retain their native scrolling.

Run the catalog locally:

```sh
cargo run --locked --example catalog
```

The catalog provides 1440 x 900 Windows, 1024 x 768 Linux, 720 x 900 macOS, and 680 x 640 minimum viewport presets. It includes Chinese and English long copy, static platform capability fixtures, and production-component interaction states. It is a design and component verification surface, not platform page code.

The dedicated Shared UI workflow owns formatting, tests, Clippy, catalog compilation, and rustdoc. Platform workflows trigger on `shared/ui/**` changes and compile their applications as consumer-integration gates without repeating the shared crate's full suite.

Mac 页面组合的离线验收在 `mac/src/app/tests/layout.rs`，通过现有 macOS `--all-targets` CI 运行，覆盖尺寸/主题/布局尺度、列表溢出和导航命令。组件目录初始明确浅色，Dark 选项与呈现一致。页面测试使用隔离运行时，不连接真实设备。

### v2 动作语义与图标来源

`Action` / `ActionTone` 区分 Accent、Neutral 和 Danger；旧 `action` 保持 Accent 便捷入口。`dialog_header` 绘制标题并返回右侧关闭按钮的响应，调用者决定只关闭哪一个弹层。组件不直接停止媒体或网络。

`assets/icons/tabler-x.svg` 来自 Frog S2 交付中的 Tabler Icons 3.49.0（MIT）；版权见同目录 `LICENSE.txt`。保留原始 SVG；运行时使用 `tabler-x.rgba` 的72×72白色RGBA像素，再按语义色着色，不引入 SVG 渲染依赖。重新生成时用 librsvg `rsvg-convert --stylesheet <含 svg { color: white; } 的 CSS> -w 72 -h 72 tabler-x.svg -o /tmp/tabler-x.png`，然后用 Pillow `Image.open(...).convert("RGBA").tobytes()` 写入RGBA文件。产物应为20736字节。

Mac 页面测试另外覆盖长播放错误时“停止并返回”可见，以及设置/详情/附近设备关闭确认中的 `×` 只关闭界面、不发送网络或媒体命令。实际网络、媒体与导出保存对话框仍需分别验证。

### v2 状态呈现

平台向 `ContextCard` / `NearbyRow` 传入 `StatusTone`，向设备行单独传入当前播放标记；共享组件不解析状态文案，也不推测连接成功。`StatusTone::indicator` 提供稳定圆点、等待进度或警示符，`notice` 承载可换行说明。`ActionTone::Primary` 为开始/观看提供轻量强调背景，选源/返回/取消保持中性。示例和所有v2调用点已同步，旧平台组件接口不变。

### 2026-10-19 第二版原型接入

Mac 播放页直接使用窗口剩余空间，不再套 Nearby 页脚或卡片；底部工具栏64pt。设置行在宽度不少于360pt时为56pt；更窄时改为84pt上下排列，操作保留明确标签。选源仍由平台系统选择器完成，组件不制造设备、媒体或成功状态。

字体通过 `include_bytes!` 内嵌 Noto Sans SC，可离线显示中文、英文和数字。该可变字体默认 `wght=100`，`install_ui_font` 明确覆盖到400；有语义的标题/按钮可以继续设置500/600。禁止因普通 Label 未指定字重而回到极细字形。字体来源和 SIL OFL 见 `assets/fonts/`，打包保留许可证。等宽日志继续以 egui 自带等宽字体为主，中文使用后备字体。

品牌入口 `v2::brand_mark` 使用 `assets/brand/moqcast.png` 的无损RGBA解码产物；主要动作字号14pt、字重500，测量和绘制复用同一文字样式。

## 版本与迁移

- 组件继续在本仓库用 path 依赖管理，`publish = false`；不另建私有 registry。
- 版本按可交付的组件迭代递增，不随每次 UI 调整递增。新增可选组件记 minor，兼容修复记 patch；0.x 的破坏性 API 变更升 minor 并写迁移说明。
- 修改包版本时同步本库及三端消费者 Cargo.lock；Desktop 应用版本与组件版本互不代替。
- v2 逐端接入。旧入口在消费者迁移、验证完成前保留；版本更新不能视为三端 UI 已完成迁移。
- 平台拥有状态投影、权限、系统选源、日志动作与媒体生命周期。公共组件只提供布局、呈现和交互结果。

| 消费端 | 当前候选 | 剩余适配 |
| --- | --- | --- |
| macOS | v2 页面及公共播放器布局 | 本次共享变更回归与原生验收 |
| Linux | v2 页面候选及公共播放器布局 | Linux 原生构建、Portal/X11 与窗口布局验证 |
| Windows | v2 页面、WGC 来源页和公共播放器布局候选 | Windows 原生构建、DPI、WGC 与媒体回归 |

### 共享页头与来源控件

`v2::header` 统一品牌、返回、截断标题与设置入口，仅返回点击结果。`source_row` 提供48pt来源行、截断与完整名称提示。`source_list` 仅包裹来源项，浮动滚动条在列表交互时出现、离开后淡出，预留固定槽位以免遮挡文字；列表之外的刷新、设置和共享动作不随列表移动。`settings_row` 在窄弹层将控件移到标签下方。`v2::PLAYER_VOLUME_CONTROL_WIDTH` 为148pt，与40pt静音按钮、8pt间距和100pt滑块一致；调用端必须使用对应代际常量预留空间。

Linux 的 Portal 选源、X11 显示器列表与 Windows 的 WGC 屏幕/窗口列表由各平台保留；共享组件不实现来源识别或后台重试。Windows 系统音频沿用既有行为，不增加无效开关。

### 2026-10-24 状态与交互验收目录

`cargo run --manifest-path shared/ui/Cargo.toml --example v2_catalog` 展示发现、连接中、失败、验证失败、已连接、可观看、准备播放、播放中和停止中的独立行样例。可切换浅深主题、空列表、40行滚动、Nearby关闭等待和媒体槽占用；按钮事件显示在目录底部，不创建网络或媒体对象。

共享组件测试验证浅深主题及当前行/普通行的详情与观看命中隔离、禁用观看不产生动作；平台页面回归负责实际快照到按钮权限和播放高亮的映射。目录样例不替代平台权限测试，宿主 egui 测试不代替目标平台真机验收。此次不增加共享 UI 的业务依赖或公共 API。
