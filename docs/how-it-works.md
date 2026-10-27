# 实现原理

本文对应当前实现。项目的目标是打开一个路径的父文件夹并选中该条目；它不是打开文件本身。

## 调用路径

CLI 在 [src/main.rs](../src/main.rs) 读取参数和环境变量，然后调用库中的 `run`。`run` 负责参数个数、路径验证、错误文本和退出码；shell 通过 `Revealer` trait 注入。测试可以用 `RecordingRevealer` 替代真实 Explorer。

生产路径在 [src/lib.rs](../src/lib.rs)：解析并归一化输入、初始化或采用当前线程 COM apartment，再查找已打开的 Explorer 视图。视图查找和标签操作在 [src/automation.rs](../src/automation.rs)，Win32 声明和官方绑定选择在 [src/ffi.rs](../src/ffi.rs)。

## 路径与 PIDL

路径按 UTF-16 交给 `SHParseDisplayName`，不构造 `explorer.exe /select,...` 命令行。解析得到完整 PIDL 后，`ILFindLastID` 取最后一个子项，`ILRemoveLastID` 将父 PIDL 截短，随后传给 `SHOpenFolderAndSelectItems`。内存通过 shell 的 `ILFree` 释放。

`resolve` 根据调用者的工作目录绝对化相对路径，检查存在性和父目录。进入 shell namespace 前，`normalize` 处理三类 Windows 路径：

| 输入 | 处理 | 原因 |
| --- | --- | --- |
| `\\?\C:\...` | 去掉 verbatim 前缀 | 本机 `SHParseDisplayName` 拒绝该形式 |
| `\\?\UNC\server\share\...` | 还原为普通 UNC 形式 | shell parser 对 verbatim UNC 同样有限制 |
| `C:relative` | 以 `C:\` 为基准锚定 | Explorer 的盘相对当前目录与调用者进程 cwd 不同 |

普通绝对路径、普通 UNC 路径和已由 cwd 解析的相对路径不做额外重写。设备命名空间形式不保证可被 shell 表示，会原样交由 shell 并按 HRESULT 报告结果。

## Explorer 视图与标签

算法先尝试当前已打开的 Explorer 视图：

1. 通过 `Shell.Application.Windows` 找到父目录匹配的 `FolderView`，在其项目中定位目标条目并调用 `FolderView.SelectItem`。
2. 若目录存在于后台标签，按 `Switch` 策略处理：`NewTab` 请求 Explorer 将目标标签带到前台，可能产生重复标签；`Overwrite` 将当前可见标签导航到目标目录。
3. 若目录未打开，调用 `SHOpenFolderAndSelectItems`，再轮询新视图并补做选择。

Windows 11 24H2 build 26100 的观测显示：shell 可能在新窗口时丢弃选择请求；后台标签的选择可以成功但不可见；窗口可最小化或位于后方。因此成功路径还会恢复并置前窗口。选中补偿属于 best effort：文档化的 shell 调用成功后，即使补偿失败，也不将已经成功交给 shell 的请求改报失败。

标签枚举使用窗口句柄和标题栏推断可见标签。`Shell.Windows` 每个标签各返回一个记录，但不提供活动标签字段；代码只把标题以标签名开头且后续是空格的记录判作可见，避免路径前缀相似造成误判。每轮轮询只枚举一次窗口，间隔从 25 ms 退避至 200 ms，并受每阶段墙钟预算约束。

## COM 与调用阻塞

`ShellRevealer` 在调用线程执行 COM 操作。它会调用 `CoInitializeEx`；若线程已初始化为另一种 apartment，`RPC_E_CHANGED_MODE` 被视为可继续使用已有 COM，而不是 shell 失败。只有本次成功取得初始化计数时才调用 `CoUninitialize`，不会释放宿主拥有的 apartment。

等待 Explorer 刷新使用 `thread::sleep`，不会泵 GUI 消息。已打开视图查找最多等待 2 秒；发起打开后等待新视图和补选最多等待 3 秒。两个阶段在少见情况下可能连续发生，因此总阻塞时间可能接近 5 秒。常见暖路径为数十毫秒，冷路径、标签切换或刷新延迟可能达到秒级。GUI 宿主应将操作放到工作线程。

## 两种 FFI 形态

默认构建由 [src/ffi/native.rs](../src/ffi/native.rs) 声明 7 个 shell32/ole32 函数。`official-bindings` feature 由 [src/ffi/official.rs](../src/ffi/official.rs) 转调 `windows` crate 的生成绑定。`src/ffi.rs` 在两种形态间选择同一组内部签名，因此业务逻辑只有一份。

启用官方 feature 的测试仍编译 native 模块，以便 [src/crosscheck.rs](../src/crosscheck.rs) 将手写声明与官方 API 对照；官方 feature 的非测试构建不编译 native 模块。两种绑定共享 Explorer 自动化层，其中 IDispatch、VARIANT 和调用封装仍由项目手写。

## 选中状态

`FolderView.SelectItem` 使用 `SVSI_SELECT | SVSI_DESELECTOTHERS | SVSI_ENSUREVISIBLE | SVSI_FOCUSED`。不能把 `SVSI_EDIT` 一并设置：该标志与 `SVSI_SELECT` 位值重叠，会导致目标进入重命名状态，而不是只高亮选中。

更多真实环境结论见 [research.md](research.md)，覆盖情况见 [testing.md](testing.md)。
