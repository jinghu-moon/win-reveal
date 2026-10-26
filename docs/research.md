# 调研与实测结论

回答两件事：**别人怎么做的**，以及**这台 Windows 11 24H2 上什么是真的**。
本地源码：`rust_sources/{showfile,win-desktop-utils,ublx}`；
`martensite-share-platform` 与 `win_open` 本地缺失，已从 crates.io 下载 `.crate` 并解压到
`rust_sources/{martensite-share-platform,win-open}`（版本 0.22.0 / 0.1.2）。
取源脚本见 `download-rust-repos.ps1`。

## 现有实现的对比

| 项目 | 底层实现（源码位置） | 错误模型 | 依赖 / unsafe | 结论 |
|:--|:--|:--|:--|:--|
| showfile | `SHParseDisplayName` + `SHOpenFolderAndSelectItems(idlist, None, 0)`，`src/lib.rs:160-169`；verbatim 前缀剥离 `src/lib.rs:140-158` | **返回 `()`，无错误通道**；COM 初始化失败直接 panic（`src/lib.rs:136`） | `windows 0.52`；有 unsafe；跨平台 cfg + `rustbus/zbus/gio` feature 门与 `compile_error!`（`src/lib.rs:36-41`） | 语义最接近，但无错误通道，无法满足"清晰错误 + 非零退出码"；它的 verbatim 前缀处理是本项目唯一采纳的做法，并补上了 `\\?\UNC\` 与盘相对（`C:a.txt`）两种它没覆盖的形态 |
| win-desktop-utils | `Command::new("explorer.exe").arg("/select,").arg(path)` **两个 token**，`src/shell.rs:352-355` | 有类型化 `Result`（`src/error.rs`） | `windows 0.62` 6 组 API、7 个 feature 模块（`Cargo.toml:34-50,143-153`） | 双 token 形态本机实测**不选中**；为 1 个函数引入整库不划算 |
| ublx | `canonicalize()` 后 `format!("/select,{p}")`，`src/modules/opener.rs:97-99` | `io::Result` | TUI 应用（rusqlite/ratatui 等），非库 | `canonicalize` 产生 `\\?\` verbatim 前缀再拼命令行，属已知坑；不可作为依赖 |
| martensite-share-platform | `explorer /select,{}` 单 token + `.status()`，`src/windows.rs:47-57`；文档明确"explorer 退出码不可靠，成功也常为 1"（`src/windows.rs:6-9`） | `ShareReply` 枚举 | 无 unsafe；但包在 `ShareBackend` 共享抽象里 | 结论正确（退出码不可信），但抽象面大于需求；`.status()` 会阻塞 |
| win-open | 默认走 `cmd/pwsh/nu start`（**只打开不选中**）；`shellexecute` feature 下 `SHOpenFolderAndSelectItems(folder, Some(&[folder]), 0)`，`src/lib.rs:417-422` | 自定义 `Error` | 默认零依赖；有 unsafe | 把目录自身 PIDL 当作待选中子项传给 API，语义错误且 `ILFree` 泄漏；文件走 `ShellExecuteExW`＝打开文件而非选中；`detect_shell` 会 spawn pwsh/nu 探测并可能 panic（`src/lib.rs:259-266`） |

## 本机实测

Windows 11 24H2，build 26100。判定手段：UI Automation 的 `SelectionItem.IsSelected`，
并以 Shell COM `FolderView.SelectItem` 作阳性对照验证判定器有效。

- `explorer.exe` 成功时也返回退出码 1 → 退出码不可用于成功判定（与 martensite 文档一致）。
- `explorer /select,` 单 token 与双 token 两种形态：目录打开、**均不选中**。
- `SHOpenFolderAndSelectItems`（两种形态）返回 `S_OK`、目录打开；新建窗口时选中被该 shell 丢弃。
- 直接引用真实 `showfile` crate（`windows` crate 的正确绑定）结果相同 → 排除自写 P/Invoke 的问题，
  确认是**本机 shell 的环境性缺陷**（新建窗口时丢弃选中请求），而非实现选型问题。
- 阳性对照：对**已打开**的视图用 Shell COM `FolderView.SelectItem` 可稳定产生高亮 →
  缺陷只发生在"新建窗口"这一条路径上，因此本工具的补偿走同一条路；
  补偿加入后，opt-in 测试在**新建窗口**场景下断言 `IsSelected=True` 通过。
- 后台标签的 ListItem **同样留在 UIA 树里**（两个同目录标签都报 `IsSelected=True`），
  所以 UIA 只能证明"选中落在某个标签上"，证明不了"用户看得见"；可见性判定只能靠标题栏前缀。
- `Navigate2` 打在**当前可见标签**上（`overwrite` 策略）实测：总标签数不变、目标目录 +1 个标签、
  原先在看的标签 −1 个、标题栏换成目标目录、条目高亮且没有进入重命名态。

因此本工具的成功判据取"shell 调用返回成功 HRESULT"（OS 契约内唯一可验证信号），
而"选中高亮"由补偿逻辑负责，并被 opt-in 真实弹窗测试断言（见 [testing.md](testing.md)）。
这些事实如何决定 `src/automation.rs` 的形状，见 [how-it-works.md](how-it-works.md)。
