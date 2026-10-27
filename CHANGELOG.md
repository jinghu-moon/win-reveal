# 变更日志

分节词汇用「新增 / 变更 / 修复 / 已知限制」。版本规则在文末，发布顺序与校验记录在
[docs/testing.md](docs/testing.md) 与 [README.md](README.md)。

## 0.1.0 — 2026-10-27

首个版本，同时是库和命令行，**仅 Windows**。行为、代价与时延都在 build 26100 上实测，
其他 Windows 版本上的表现由工具自己兜住（选中补偿会退化成"只打开目录"）。

### 新增

- **库面**：`trait Revealer`、`ShellRevealer`、`ShellRevealer::reveal_with(path, Switch)`、
  `RecordingRevealer`（满足 `Send + Sync`，宿主自己的测试不必弹资源管理器）、`enum Error`
  （实现 `std::error::Error`，`Shell { step, hresult }` 带出失败的 Win32 步骤与原始 HRESULT）、
  `resolve(raw, cwd)`、`run(args, cwd, revealer, stderr)`、`EXIT_OK` / `EXIT_SHELL` / `EXIT_USAGE`、
  `USAGE`、`DRY_RUN_VAR`、`dry_run_setting()`。
- **命令行**：`reveal <file-or-folder-path>`，恰好一个参数；`--help` / `-h` 只认作第一个参数。
  退出码 0 成功 / 1 shell 调用失败 / 2 输入错误。
- **配置项**：`REVEAL_TAB_POLICY`（`newtab` 默认 / `overwrite`）与 `REVEAL_DRY_RUN`；
  写法不认识一律退出 2 并在 stderr 说明，不静默按默认走。
- **两种构建形态**：默认零第三方依赖（自声明 7 个 `extern "system"` 函数）；
  `--features official-bindings` 走微软生成的 `windows` 0.62.2 绑定。同一套算法、同一份行为、
  同一量级时延，产物里都不含第三方代码。
- **宿主适配**：调用线程已经有 COM 套间（包括按 MTA 初始化过的）时沿用它，且绝不替宿主
  `CoUninitialize`；等 shell 的三段轮询用 25 ms 起步、×1.5、200 ms 封顶的退避，每轮只遍历一次
  `Shell.Windows`。
- 路径形态归一化：`\\?\C:\…`、`\\?\UNC\server\share\…`、盘相对 `C:a.txt` 在交给 shell 命名空间
  之前改写；带空格、逗号、中文的路径不经过命令行解析。
- 文档：README 之外的
  [docs/how-it-works.md](docs/how-it-works.md)、[docs/research.md](docs/research.md)、
  [docs/testing.md](docs/testing.md)、[docs/latency.md](docs/latency.md)、
  [docs/manual-verification.md](docs/manual-verification.md)。

### 已知限制

- 非 Windows 目标编不过去，这是设计而不是缺陷：crate 里有一条 `compile_error!`。宿主用
  `[target.'cfg(windows)'.dependencies]` 即可让 Linux/macOS 构建完全不解析这个 crate。
- 正因为上一条，docs.rs 构建不出本文档站的页面。本地 `cargo doc --no-deps`，或直接读仓库里的
  README 与 `docs/`。
- 退出码 1（shell 调用失败）没有真机测试覆盖——要稳定构造 `SHParseDisplayName` 失败太难，
  权衡见 [docs/testing.md](docs/testing.md)。
- 时延地板约 18 ms，在你正看着的文件夹里换选一个文件约 50–60 ms；目标目录没开过、条目刚写入、
  或目标在后台标签时是 0.4–1.3 s，因为代价花在**等 shell**，并随你打开的标签数线性增长。
- `overwrite` 策略会把你正在看的标签导航到目标目录（换取"标签数不增长"）；默认 `newtab` 不动那个
  标签，但实测代价是为目标目录多出一个重复标签。

## 版本策略

- **0.x**：README「作为库使用」那张表里的东西就是公开面。动它（改名、改签名、改语义、删掉）
  算 breaking，升次版本号；同一 `0.x` 系列内的修订只可能是 bug 修复、文档与时延数字。
  `cargo update` 不会跨次版本号自动升级，所以 breaking 不会在没人看见时发生。
- **1.0.0 起**：遵循 Cargo 社区的 semver 惯例，breaking 升 major。
- **MSRV**（当前 `1.88`）：只写本机实测过的最低工具链。提高 MSRV 算 breaking（`0.x` 升次版本，
  1.0 之后升 major）；降低不需要，但也只在实测之后才写进元数据。
- **feature `official-bindings`** 与 `[[bin]] reveal`、`REVEAL_*` 环境变量前缀都在公开面里，
  受同样约束。
