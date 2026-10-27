# Changelog

本文件记录面向用户的功能、行为变化和已知限制。日期采用首次发布日期。

## 0.1.0 - 2026-10-27

首个版本。提供 Windows-only 的 `reveal` 命令行程序和 Rust 库。

### Added

- `reveal <file-or-folder-path>`：打开目标所在目录并选中目标。
- 库 API：`Revealer`、`ShellRevealer`、`ShellRevealer::reveal_with`、`RecordingRevealer`、`Switch`、`Error`、`resolve` 和 `run`。
- `REVEAL_TAB_POLICY`，支持默认 `newtab` 和 `overwrite` 策略。
- `REVEAL_DRY_RUN`，用于在不调用 Explorer 的情况下验证路径处理。
- 默认手写 Win32 FFI，以及 `official-bindings` feature 提供的 `windows` crate 绑定形态。
- 对 verbatim disk/UNC 与盘相对路径的归一化。
- COM apartment 所有权处理：沿用调用线程已有的 apartment，不释放宿主拥有的初始化计数。

### Behavior

- 路径必须存在并且有父目录；相对路径按调用进程的当前工作目录解析。
- CLI 退出码为 `0`（请求交给 shell）、`1`（shell HRESULT 失败）和 `2`（输入或设置无效）。
- 默认 `newtab` 保留当前可见标签，但后台目标可能产生重复标签；`overwrite` 复用当前可见标签并替换其内容。
- 真实 Explorer 自动化用于补选并置前。不同 Windows shell 版本可能不暴露所需文件夹视图，届时可能只打开目录。

### Known limitations

- 仅支持 Windows；非 Windows 目标在编译时失败。
- shell 操作可能阻塞调用线程。GUI 宿主应在工作线程调用。
- docs.rs 使用的非 Windows 构建环境无法构建此 crate；使用仓库 README 和 `docs/` 中的文档。
- shell 失败 HRESULT 的真实系统错误难以稳定构造，主要通过单元测试验证错误映射。

## Versioning policy

在 0.x 阶段，公开 Rust API、`reveal` 二进制、`official-bindings` feature 和 `REVEAL_*` 环境变量均属于兼容性承诺。破坏性变化提升次版本号。MSRV 当前为 Rust 1.88；提高 MSRV 视为破坏性变化。
