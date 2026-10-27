# 调研与平台实测

本文记录实现选型和 Windows 11 24H2 build 26100 上的观察。Explorer 是异步、状态化的系统组件，其他 Windows 版本或 shell 配置可能表现不同；本机测量不是跨版本保证。

## 选型

需求是“打开父文件夹并选中条目”，不是打开文件。比较过的路径如下：

| 方式 | 观察 | 结论 |
| --- | --- | --- |
| `explorer.exe /select,<path>` | 属于未文档化命令行约定；单 token、双 token 在本机均未选中，成功时进程退出码也可能为 1 | 不用于主调用 |
| `SHOpenFolderAndSelectItems` | 文档化的打开并选中入口，返回 HRESULT；路径经 PIDL 而非命令行传入 | 主调用 |
| `FolderView.SelectItem` | 对已存在的文件夹视图可补做选择；对后台标签的选择不代表用户可见 | Explorer 自动化补偿 |
| `explorer.exe <folder>` | 可将已存在的后台目录带到前台，但本机可能新建重复标签 | 仅用于 `newtab` 标签策略 |
| `Navigate2` | 可将当前可见标签导航到目标文件夹，不增加标签总数 | 用于 `overwrite` 策略 |

调用 `SHOpenFolderAndSelectItems` 时，完整 PIDL 被分成父 PIDL 和最后一个子 PIDL。用系统命令拼接路径会引入引号、空格和逗号解析差异，因此不作为主要接口。

## 对照实现

调研过 `showfile`、`win-desktop-utils`、`ublx`、`martensite-share-platform` 和 `win-open`：

- `showfile` 使用相近的 Shell API，但错误反馈不足；其 verbatim path 处理提供了本项目路径归一化的参考。
- `win-desktop-utils` 提供较完整的 Windows 工具库，但为单个 reveal 操作引入的 API 和依赖面较大。
- `ublx` 与 `martensite-share-platform` 使用 Explorer 命令行；后者关于 Explorer 退出码不可靠的判断与本机观测一致。
- `win-open` 的默认行为是打开文件/目录，不等同于在目录中选中目标；其可选 shell 路径与本需求语义不同。

调研用第三方源码曾放在 `rust_sources/`，该目录被 `.gitignore` 排除，不随仓库提交或 crate 打包。来源与获取脚本为 `download-rust-repos.ps1`。

## build 26100 观测

- `explorer.exe` 即使成功打开目标，也可能退出码为 1，因此不能用其退出码判断成功。
- `/select,` 单 token 和分开的双 token 在本机均打开目录但未选中条目。
- `SHOpenFolderAndSelectItems` 返回 `S_OK`，但新建视图时 shell 丢弃了原始选择；对已打开视图使用 `FolderView.SelectItem` 可产生高亮。
- 后台标签仍能通过 Shell COM 访问；UI Automation 也可能把后台标签条目标记为 selected，所以仅凭 `IsSelected` 无法判断用户是否看见它。
- 标题栏显示当前可见标签的名称，因此实现用标题栏与 Shell COM 记录联合推断可见标签。
- `Navigate2` 导航当前可见标签时，实测总标签数不变，目标目录标签数加一，原前台目录标签数减一。
- `\?\` verbatim 路径形式在常规文件存在性检查后仍可能被 `SHParseDisplayName` 以 `E_INVALIDARG` 拒绝；普通盘路径和 UNC 路径可解析。
- 将 `SVSI_EDIT` 与 `SVSI_SELECT` 混用会进入重命名态；实现只使用选择、取消其他选择、确保可见和焦点标志。

## 证据边界

这些结果来自一台 Windows 11 24H2 build 26100 机器及相应 UI Automation / Shell COM 探针。它们用于解释实现和测试设计，不保证所有 Windows 版本、语言包、Explorer 设置或替代 shell 都有相同行为。遇到只打开目录但没有高亮时，按 [manual-verification.md](manual-verification.md) 的第 8 节区分补偿未匹配与 shell 本身不支持。

测试如何验证两种 FFI 形态，以及哪些行为需真实 Explorer，见 [testing.md](testing.md)。
