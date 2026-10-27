# 测试说明

## 常规验证

```powershell
cargo test
cargo test --features official-bindings
cargo clippy --all-targets
cargo clippy --all-targets --features official-bindings
cargo doc --no-deps
cargo doc --no-deps --features official-bindings
cargo +1.88.0 check --all-targets
cargo +1.88.0 check --all-targets --features official-bindings
cargo +1.88.0 test --doc
```

当前两种绑定形态均为 35 个库单元测试、19 个 CLI 集成测试、3 个 doctest 通过；3 个涉及真实 Explorer 或时延测量的测试默认忽略。

## 不操作桌面的测试

`Revealer` 是 CLI 与 shell 之间的注入边界。单元测试使用 `RecordingRevealer`；真实 CLI 集成测试通过 `REVEAL_DRY_RUN` 验证路径解析与输出，不弹资源管理器。

`src/tests.rs` 包含库内部测试。它作为 `src/lib.rs` 的子模块编译，可以访问私有实现而不扩大公开 API。测试覆盖参数校验、路径解析和归一化、错误映射、配置解析、COM apartment 所有权，以及实现的 `Send + Sync` 属性。`src/automation.rs` 的纯逻辑测试覆盖路径与 Explorer 标题匹配。

`tests/cli.rs` 启动真实二进制，检查退出码、标准输出/错误和环境变量行为。测试会清除继承的 `REVEAL_*` 设置，避免运行环境改变结果。

## FFI 交叉校验

`src/crosscheck.rs` 在 `cargo test` 时编译，生产构建不包含它。它始终直接调用 `ffi::native` 并与 `windows` crate API 对照，即使测试启用了 `official-bindings`，避免两边实际调用同一实现。

当前 4 条交叉测试覆盖：

1. 官方 API 与手写声明对 verbatim 路径给出相同 HRESULT，归一化后的路径两边均可解析。
2. 手写 `ILFindLastID` / `ILRemoveLastID` 对官方解析出的 PIDL 正确拆出父项和子项，且内存可由共享的 shell allocator 释放。
3. `SHOpenFolderAndSelectItems` 对空 folder 和空数组在两边均返回 `E_INVALIDARG`。此参数组合已在目标系统验证不会打开 Explorer 窗口。它验证两个底层 API 的空参数路径，不覆盖 `ffi::official` 的非空数组包装分支。
4. 两边的 `ILFree(NULL)` 都安全返回。

官方包装在 `apidl` 非空时会从原始指针构造 slice。尝试用空 folder 加真实 PIDL 进入该分支时，shell 会先读取 PIDL，再处理 folder；观察到测试进程执行耗时异常且结果不稳定，因此没有将其放入默认套件。该包装分支目前由官方绑定形态下的真实 Explorer 测试间接覆盖；默认 UI 测试忽略，故这不是每次 `cargo test` 都执行的契约测试。

## 真实 Explorer 测试

这些测试会创建临时目录、打开或移动 Explorer 标签，并可能在测试后留下标签。确认可以操作当前桌面后，串行运行：

```powershell
$env:REVEAL_UI_TEST = "1"
cargo test --test ui_reveal -- --ignored --test-threads=1 --nocapture
```

`tests/ui_reveal.rs` 有两项测试：

- reveal 后通过 UI Automation 检查目标被选中、没有进入重命名编辑状态；复用同一目录时不增加视图，并验证最小化的窗口会恢复。
- 在 `overwrite` 策略下检查目标选中、可见标签切换、总标签数不变，以及原前台目录标签被替换。

UI Automation 中后台标签的条目也可能显示为 selected，因此测试还检查 Explorer 标题栏，以判断目标是否实际位于可见标签。测试会尽量清理只含夹具的窗口；与用户标签共用的窗口不会整体关闭。

## 时延测试

`tests/latency.rs` 也会操作 Explorer。运行方式与数据见 [latency.md](latency.md)：

```powershell
$env:REVEAL_LATENCY_TEST = "1"
cargo test --test latency -- --ignored --nocapture
```

## 最近验证记录

2026-10-27 的当前源码复验：两种 feature 形态各自通过 35 个库测试、19 个 CLI 测试和 3 个 doctest；clippy 与文档构建通过。拆出 `src/tests.rs` 后，两种 release exe 仍为 297,984 B。当前 crate 包含 24 个文件，`cargo publish --dry-run` 在隔离目录重新编译成功。crate 字节数会随文档变化，不作为稳定基线。

真实 Explorer 测试与时延测试未在最近一批测试文件整理和文档更新中重跑；生产路径未因测试文件搬迁而变化。上一次真实 Explorer 测试记录和时延样本应结合当前 Windows build 与桌面状态重新验证，不作为本次运行结果。
