# win-reveal

Windows 命令行工具兼库：传入一个文件（或目录）路径，在资源管理器中打开其所在目录并选中该项。

```text
reveal.exe "C:\some dir\文件.txt"
```

包名带 `win-` 限定（crates.io 上 `reveal` 已被一个图片查看器占用，`windows-*` 前缀实际属于
微软 windows-rs 家族），而**命令仍然是 `reveal`**：那是人要敲的东西，平台限定由包名承担。
`[[bin]] name = "reveal"` 与包名解耦，改名不动任何调用习惯。

```toml
[dependencies]
win-reveal = "0.1"                    # 库用法：win_reveal::run / Revealer / ShellRevealer
```

仅支持 Windows（非 Windows 目标直接 `compile_error!`），无跨平台抽象；不引入 CLI 框架，
参数解析只用 `std::env::args_os`。

**两种构建形态，同一套算法**，差别只在 shell 绑定的来源（见 [src/ffi.rs](src/ffi.rs)）：

| 形态 | 命令 | shell 绑定 | 产物（每次构建后复制留档） |
|:--|:--|:--|:--|
| 零依赖（默认） | `cargo build --release` | `extern "system"` 自声明 7 个函数，只链接 `shell32`/`ole32` | `reveal-native.exe`，297,472 B |
| 官方绑定 | `cargo build --release --features official-bindings` | 同名包装，转调 `windows` crate 0.62.2 | `reveal-official.exe`，298,496 B |

feature 不叫 `windows`：在一个整体仅 Windows 的 crate 上，`--features windows` 读起来像
"默认构建还不支持 Windows"，而它真正说的是"绑定换成微软生成的那套"。

默认形态的 `cargo tree` 里没有任何 normal 依赖（`windows` 只出现在 `[dev-dependencies]`，
用于测试期的交叉校验）。官方形态同样不引入运行时第三方代码：调的还是那两个系统 DLL，
只是绑定由微软生成，产物大 1,024 B、编译期多出 `windows-core` 等 crate。被本项目依赖时按
需要选一种即可，两者的可观察行为与退出码完全一致。

## 退出码

| 码 | 含义 |
|:--|:--|
| 0 | 已成功把"打开目录并选中"请求交给 shell；`--help`/`-h` 也走这一档 |
| 1 | shell 调用本身失败（stderr 给出失败的 Win32 步骤与 HRESULT） |
| 2 | 输入错误：参数个数 ≠ 1、空参数、路径不存在、路径无父目录、两个配置项的取值不认识（stderr 给出原因 + usage） |

`reveal.exe --help`（或 `-h`，只认作**第一个**参数）把用法、两个配置项与退出码打到 stdout 后
退出 0；`--helpme` 之类的写法仍按路径处理，`reveal.exe 那个文件 --help` 则是"参数个数 2"。

`REVEAL_DRY_RUN` 取 `1`/`true`/`yes`（不区分大小写）时不触碰 shell，只在 stdout 打印
`would reveal: <解析后的绝对路径>`，用于端到端测试与排查；`0`/`false`/`no`/空值表示关——
一个只能被打开、关不掉的开关，在被 export 进 shell 会话之后就是事故源。其它写法直接退出 2，
不静默按默认走。

## 配置项：目标目录已在后台标签时怎么办

`REVEAL_TAB_POLICY=newtab|overwrite`（默认 `newtab`，取值不区分大小写；其它写法直接报错退出 2，
不静默按默认走）。它只影响一种局面：**目标目录已经开着，但在你正在看的标签之外**。

| 起始状态 | `newtab`（默认） | `overwrite` |
|:--|:--|:--|
| 目标目录就是当前可见标签 | 只选中并置前 | 同左 |
| 目标目录没在任何标签开过 | shell 开一个新标签并选中 | 同左 |
| 目标目录在**后台标签** | 你正在看的标签**不动**，把目标标签带到前台并选中；实测代价是会为该目录多出一个标签（重复 reveal 会累积标签） | 把你正在看的标签**导航**到目标目录：不新增标签，代价是那个标签原来的内容被替换 |

## 实现选型（第一性原理 + 奥卡姆剃刀）

不可再还原的事实只有三条：

1. Windows 上"打开目录并选中条目"的**文档化**入口只有一个：`SHOpenFolderAndSelectItems`，
   它返回 HRESULT，因此失败是可判定的。
2. `explorer.exe /select,<path>` 是未文档化的命令行约定，其退出码无意义（本机实测成功也返回 1），
   且路径要经过一次命令行拼接，空格/逗号/引号都是真实的解析风险。
3. 路径一旦走 shell 命名空间（UTF-16 PIDL），就完全不经过命令行，转义问题从根源上消失。

由这三条直接推出：调用 `SHOpenFolderAndSelectItems`，路径以 UTF-16 传入，用 HRESULT 决定退出码。
其余一切（CLI 框架、跨平台层、canonicalize、命令行拼接）都是不必要的实体，全部不引入。
唯一保留的一次输入改写是路径形态归一化——它不是抽象层，而是下面那条实测约束的直接结果。

### 路径形态归一化（`normalize`，`src/lib.rs:244`）

`resolve` 在把路径交给 shell 命名空间之前改写三种写法，全部是 build 26100 上的实测结论：

| 输入 | shell 命名空间的反应 | 归一化为 |
|:--|:--|:--|
| `\\?\C:\a.txt` | `File::Exists` 为真，但 `SHParseDisplayName` 返回 `E_INVALIDARG`（`0x80070057`） | `C:\a.txt` |
| `\\?\UNC\server\share\a.txt` | 同上 | `\\server\share\a.txt` |
| `C:a.txt`（盘符后无 `\`） | 合法 Win32 相对路径，但基准是**资源管理器进程自己**记住的"该盘当前目录"，与调用方 cwd 无关 | `C:\a.txt` |
| `\\?\Device\Harddisk0\Partition1`、`\\.\GLOBAL\RPC` | 设备命名空间形态没有 shell 对应物 | **原样交给 shell**，让它用自己的 HRESULT 说话 |
| `C:\a.txt`、`\\server\share\a.txt`、`rel\a.txt` | 本来就正确/已由 cwd 解析 | 不动 |

前两类若不改写，工具会对一个刚验证过存在的路径报 shell 失败；第三类若不改写，同一个参数
在不同时刻会指向不同文件（而且 `cwd.join("C:a.txt")` 会把它当相对路径直接拼接，得到一个
`...\cwd\C:a.txt` 这种不存在的东西）。这条正是从 `showfile` 的 `src/lib.rs:140-158` 学到的处理，
`\\?\` 与 `\\?\UNC\` 两种形态由官方 `windows` 绑定在同一进程里交叉验证过（见"测试"一节），
确认拒绝来自 API 而不是我们的声明。

FFI 层声明 7 个函数（`shell32` 的 `SHParseDisplayName`/`SHOpenFolderAndSelectItems`/
`ILFindLastID`/`ILRemoveLastID`/`ILFree` 加 `ole32` 的 `CoInitializeEx`/`CoUninitialize`）；
`unsafe` 集中在 `src/ffi/`（声明或包装）、`reveal_parsed`（一次调用）与 `src/automation.rs`
（IDispatch 客户端），`resolve`/`run` 这些业务路径是 safe 的。PIDL 拆分采用文档化的
*parent PIDL + child 数组* 形态（`ILFindLastID` + `ILRemoveLastID`），而非 `cidl=0/apidl=NULL`
的未文档化形态。

唯一保留的命令行使用是 `newtab` 策略下的那一次 `explorer.exe <目录>`：本机 shell 里它是唯一能把
已有后台标签带到前台的手段。它只把目录路径作为**一个参数**交给 `std::process::Command`（拼接与
转义由 OS 负责），不带 `/select,` 前缀——那个前缀既不选中又会把路径按逗号重新解析，选中仍由
`FolderView.SelectItem` 完成。它的退出码照例不读（本机实测成功也返回 1）。

### 选中、标签页与置前（`src/automation.rs`）

顺序是**先复用、后打开**：

1. 先在**当前可见的标签**里选中：`Shell.Application → Windows → Document(文件夹视图) →
   Folder.Self.Path` 匹配目标父目录，在 `Folder.Items` 中按名匹配目标项并调用
   `FolderView.SelectItem`，随后把该窗口置前（含从最小化恢复）。命中就不再调用
   `SHOpenFolderAndSelectItems`。视图存在但条目尚未列出（刚创建的文件，Explorer 异步刷新）
   时最多等 10×200ms 再放弃复用。
2. 目录只存在于**后台标签**时（`Miss::Hidden`）按配置项分流：`overwrite` 对当前可见标签的
   窗口对象调 `Navigate2(目标目录)`（实测有效，等待至多 10×150ms）后正常选中；`newtab`
   **不调用**打开接口——实测对一个已经开着的目录再调 `SHOpenFolderAndSelectItems` 只会
   在后台标签里静默选中（用户看不见），于是改由 `explorer.exe <目录>` 把该标签带到前台，
   选中仍由第 1 步的 `FolderView.SelectItem` 完成。
3. 没有任何标签匹配时才调用 `SHOpenFolderAndSelectItems` 打开目录。本机 shell（build 26100）
   在**新建**窗口时丢弃它的选中请求（见下方实测），因此打开后再走第 1 步同一条路补选中并置前，
   最多重试 15×200ms 等待窗口创建完成。

标签页的四个实测事实决定了上面的形状：

- 一个带 N 个标签的窗口在 `Shell.Windows` 里是 **N 条记录**（共用同一 HWND），且没有任何字段
  标明哪条是当前标签。可用信号只有标题栏：`<当前标签显示名> 和 N 个其他选项卡 - 文件资源管理器`
  （开启"在标题栏显示完整路径"时前半段是完整路径）。判定用"标题以该标签的名字**开头且随后是
  空格**"，不能用"包含"：`D:\…\p1` 会被 `D:\…\p1-keeper 和 …` 的标题命中，于是把一个后台标签
  当成可见标签，选中打在看不见的地方——用户读作"没反应"。
- 对**后台标签**调 `FolderView.SelectItem` 会真的选中，但用户看不见；`Navigate2` 打在后台标签上
  只刷新它、不切过去，`SHOpenFolderAndSelectItems` 也不切。UIA 也帮不上：标签项虽然能读到
  `IsSelected`，却没有 Invoke/SelectionItem 模式，既不能切换也不能关闭。能切换的只有
  `explorer.exe <目录>`（会多一个标签）与覆盖当前标签的 `Navigate2`，这就是上面那个配置项的由来。
- 后台标签的 `Document` 依然可用，所以判定用 `GetWindowTextW`，句柄取自
  `IWebBrowser2.HWND`：该属性以 `VT_UI8`（type code `0x14`）返回，句柄就在变体载荷里，
  用 `IsWindow` 校验后采用——按 `VT_I4` 读会全部拿到 0，这也是早先置前与可见性判断失灵的原因。
  另外 `Shell.Windows` 在无窗口时仍含"此电脑"等非 `CabinetWClass` 条目，故只接受
  `CabinetWClass` 的句柄，避免桌面/此电脑条目把 reveal 吞掉。
- 窗口**最小化**时标题仍显示它的当前标签（所以"可见"判定为真），但 UIA 里列不出任何条目，
  选中成功了用户也看不到——因此每次成功路径都要 `bring_to_front`，它负责 `SW_RESTORE` 与抢前台。

`SelectItem` 的状态位取 `SVSI_SELECT | SVSI_DESELECTOTHERS | SVSI_ENSUREVISIBLE | SVSI_FOCUSED`
（`0x1|0x4|0x8|0x10`）。这里有个真实的坑：`_SVSIF` 里 `SVSI_EDIT = 0x3` **包含** `SVSI_SELECT`，
所以把 bit0+bit1 一起置上（早期版本用的 `1|2`，以及看似无害的 `7`）等于请求"选中并进入重命名"，
用户症状就是 reveal 后文件直接进了重命名框。位值以 `ShObjIdl_core.idl` 为准。

该客户端是手写的后期绑定 IDispatch（`GetIDsOfNames`/`Invoke` + 24 字节 `VARIANT`）；
两种构建形态都保留它——`official-bindings` feature 只换掉那 7 个 shell 声明，不换这里，因为
VARIANT/`DISPPARAMS` 的构造本身就是要测的代码，交给生成绑定等于把被测对象换成了依赖库。
作为参数传出的 `BSTR` 必须是 `SysAllocString` 自己拥有的：
传借用指针会让变体析构里的 `VariantClear` 释放不属于我们的内存，进程以 `0xC0000374`
（堆损坏）崩掉。补偿失败不改变退出码：文档化 shell 调用已成功、目录已打开，补偿只负责把本机
shell 丢掉的那一次选中补回来。

## 调研结论（读了哪些源码）

本地源码：`rust_sources/{showfile,win-desktop-utils,ublx}`；
`martensite-share-platform` 与 `win_open` 本地缺失，已从 crates.io 下载 `.crate` 并解压到
`rust_sources/{martensite-share-platform,win-open}`（版本 0.22.0 / 0.1.2）。

| 项目 | 底层实现（源码位置） | 错误模型 | 依赖 / unsafe | 结论 |
|:--|:--|:--|:--|:--|
| showfile | `SHParseDisplayName` + `SHOpenFolderAndSelectItems(idlist, None, 0)`，`src/lib.rs:160-169`；verbatim 前缀剥离 `src/lib.rs:140-158` | **返回 `()`，无错误通道**；COM 初始化失败直接 panic（`src/lib.rs:136`） | `windows 0.52`；有 unsafe；跨平台 cfg + `rustbus/zbus/gio` feature 门与 `compile_error!`（`src/lib.rs:36-41`） | 语义最接近，但无错误通道，无法满足"清晰错误 + 非零退出码"；它的 verbatim 前缀处理是本项目唯一采纳的做法，并补上了 `\\?\UNC\` 与盘相对（`C:a.txt`）两种它没覆盖的形态 |
| win-desktop-utils | `Command::new("explorer.exe").arg("/select,").arg(path)` **两个 token**，`src/shell.rs:352-355` | 有类型化 `Result`（`src/error.rs`） | `windows 0.62` 6 组 API、7 个 feature 模块（`Cargo.toml:34-50,143-153`） | 双 token 形态本机实测**不选中**；为 1 个函数引入整库不划算 |
| ublx | `canonicalize()` 后 `format!("/select,{p}")`，`src/modules/opener.rs:97-99` | `io::Result` | TUI 应用（rusqlite/ratatui 等），非库 | `canonicalize` 产生 `\\?\` verbatim 前缀再拼命令行，属已知坑；不可作为依赖 |
| martensite-share-platform | `explorer /select,{}` 单 token + `.status()`，`src/windows.rs:47-57`；文档明确"explorer 退出码不可靠，成功也常为 1"（`src/windows.rs:6-9`） | `ShareReply` 枚举 | 无 unsafe；但包在 `ShareBackend` 共享抽象里 | 结论正确（退出码不可信），但抽象面大于需求；`.status()` 会阻塞 |
| win-open | 默认走 `cmd/pwsh/nu start`（**只打开不选中**）；`shellexecute` feature 下 `SHOpenFolderAndSelectItems(folder, Some(&[folder]), 0)`，`src/lib.rs:417-422` | 自定义 `Error` | 默认零依赖；有 unsafe | 把目录自身 PIDL 当作待选中子项传给 API，语义错误且 `ILFree` 泄漏；文件走 `ShellExecuteExW`＝打开文件而非选中；`detect_shell` 会 spawn pwsh/nu 探测并可能 panic（`src/lib.rs:259-266`） |

**本机实测**（Windows 11 24H2, build 26100，用 UI Automation 的 `SelectionItem.IsSelected` 作判定，
并以 Shell COM `FolderView.SelectItem` 作阳性对照验证判定器有效）：

- `explorer.exe` 成功时也返回退出码 1 → 退出码不可用于成功判定（与 martensite 文档一致）。
- `explorer /select,` 单 token 与双 token 两种形态：目录打开、**均不选中**。
- `SHOpenFolderAndSelectItems`（两种形态）返回 `S_OK`、目录打开；新建窗口时选中被该 shell 丢弃。
- 直接引用真实 `showfile` crate（`windows` crate 的正确绑定）结果相同 → 排除自写 P/Invoke 的问题，
  确认是**本机 shell 的环境性缺陷**（新建窗口时丢弃选中请求），而非实现选型问题。
- 阳性对照：对**已打开**的视图用 Shell COM `FolderView.SelectItem` 可稳定产生高亮 →
  缺陷只发生在"新建窗口"这一条路径上，因此本工具的补偿走同一条路；
  补偿加入后，opt-in 测试在**新建窗口**场景下断言 `IsSelected=True` 通过。

因此本工具的成功判据取"shell 调用返回成功 HRESULT"（OS 契约内唯一可验证信号），
而"选中高亮"由补偿逻辑负责，并被 opt-in 真实弹窗测试断言（见下）。

## 测试

- 可注入边界：`Revealer` trait；`RecordingRevealer` 记录被要求选中的路径；二进制的
  `REVEAL_DRY_RUN=1|true|yes` 换成打印型实现 → 端到端测试**不会弹出资源管理器**。
- `src/lib.rs` 单元测试 22 个：参数个数/空参/缺失、相对路径按 cwd 解析、空格+中文、逗号+`&`、
  目录输入、尾分隔符、`..`、盘根拒绝、shell 失败映射为退出码 1、`REVEAL_TAB_POLICY` 默认值/
  大小写/非法值、`REVEAL_DRY_RUN` 的开/关/非法三档映射、每条 usage 错误以 `USAGE` 常量收尾
  （`--help` 里嵌的是同一个常量），以及四种路径形态（`\\?\C:` 改写后仍走完整 reveal、
  `\\?\UNC\` 还原成 UNC、盘相对 `C:a.txt` 锚到盘根、设备命名空间形态原样不动）；外加
  `src/automation.rs` 中路径匹配与标题可见性（前缀边界、中文、空名）6 个纯函数测试。
- `src/crosscheck.rs` 2 个交叉校验（只在 `cargo test` 编译，二进制不依赖它；只解析
  PIDL，**绝不调用** `SHOpenFolderAndSelectItems`，因此不开窗口）：
  ① 同一个 `\\?\…` 路径，官方 `Shell::SHParseDisplayName` 与我们的声明返回**同一个**
  `0x80070057`，改写后两边都 `S_OK` → 归一化补的是 API 的约束，不是我们自己声明的错；
  ② 官方解析出一个 PIDL 后 `ILClone` 一份，用**我们的** `ILFindLastID`/`ILRemoveLastID`/`ILFree`
  操作它，再用官方 `ILIsParent(ours, full, true|false)` 断言截留的正是直接父级，并比较子项字节
  → 拆分算法与官方 helper 一致，且 `ILFree` 与官方共用同一个分配器。
  交叉校验始终走 `ffi::native`（不是当前形态选中的那套），否则 `--features official-bindings` 下两边
  变成同一份代码，断言就成了自证。实测还留下一条 PIDL 常识：**同一路径两次解析的字节并不相同**
  （文件夹 item id 内嵌 shell 会刷新的时间戳），所以比较只能在同一块内存上做。
- `tests/cli.rs` 19 个：真实二进制的退出码与 stderr 文案（全部在触达 shell 之前失败，或只失败在
  读取标签策略那一步），`--help`/`-h` 走 stdout + 退出 0 且与错误路径共用同一行 usage，
  `--helpme` 与出现在第二个位置的 `--help` 仍按普通参数处理，`REVEAL_DRY_RUN` 的 `0|false|NO|空` 确实关到了
  "会去读标签策略"的程度而 `1|true|YES` 确实跳过了整个 shell 分支，以及 dry-run 下的精确路径回显。
  每个用例都先 `env_remove` 掉两个配置项，避免跑 cargo 的那个 shell 里残留的变量改变观察结果。
- `tests/ui_reveal.rs` 1 个 `#[ignore]` opt-in 测试，`REVEAL_UI_TEST=1` 时真实弹窗，三段断言：
  ① 退出码 0 **且** UIA 读到 `IsSelected=True` **且**没有以该文件名为标题的 `Edit`（没进重命名态）；
  ② 同目录第二次 reveal，断言视图数 ≥1 且不变（视图数断言前先校验非 0，否则"没多开窗口"会因为
  查询本身失效而虚假通过）；③ 把承载夹具的窗口**最小化**后第三次 reveal，断言窗口不再最小化
  且文件高亮——这一步专门守着"窗口看不见时选中不可见"这一类回归。收尾 `WM_CLOSE` 只关那些
  标签全部属于夹具的窗口（共用 HWND 的窗口里有用户自己的标签）。
  该测试会在当前资源管理器窗口里留下标签（shell 会把新目录并入已有窗口），跑完请手动 `Ctrl+W`。

基线：仓库原本没有 Cargo 工程（`cargo test` → `could not find Cargo.toml`），无既有功能可回归。
最近一次全量复验（`--help` 与 `REVEAL_DRY_RUN` 取值校验之后，两种形态各跑一遍，结果相同）：
`cargo test` 与 `cargo test --features official-bindings` → 49 passed, 0 failed, 1 ignored；
`cargo clippy --all-targets`（含 `--features official-bindings`）零警告；
`REVEAL_UI_TEST=1 cargo test --test ui_reveal -- --ignored` 在两种形态下各 → 1 passed。

## 手动验证步骤（真实弹窗与选中）

以下命令在 PowerShell 里用 `& 'path' 'arg'; "exit=$LASTEXITCODE"` 取退出码；
`… & echo exit=%ERRORLEVEL%` 会被 PowerShell 解析成后台 Job，取不到退出码。

1. `cargo build --release`（零依赖形态，产物即 `target\release\reveal.exe`）；
   换成官方绑定形态用 `cargo build --release --features official-bindings`，下面每一步对两者结果相同。
   想同时留下两份可对比的文件，构建后各自复制一份（`reveal-native.exe` / `reveal-official.exe`）。
2. 新建夹具（放项目内的 `tmp\`，免得 `C:\` 根需要提权）。**直接在控制台粘贴，别存成 `.ps1`**：
   PowerShell 5.1 读无 BOM 的脚本会按 ANSI 解码，中文目录名会变味。

   ```powershell
   mkdir 'tmp\reveal-check\子 目录, x' -Force
   Set-Content -LiteralPath 'tmp\reveal-check\子 目录, x\目标 文件.txt' -Value 'x'
   $p = (Resolve-Path -LiteralPath 'tmp\reveal-check\子 目录, x\目标 文件.txt').Path
   ```

3. 运行：`& '.\target\release\reveal.exe' $p; "exit=$LASTEXITCODE"`
4. 预期：`exit=0`；资源管理器打开 `子 目录, x` 且 `目标 文件.txt` **高亮选中而非进入重命名**；
   窗口被置前（原先最小化也会恢复）；stderr 无输出。命令返回时高亮应已生效（补偿最多等待 3 秒）。
   若该目录已在**当前可见标签**中打开，则不会新增标签也不会导航。
5. 四种起始状态逐一验证（默认 `newtab`）：
   - 资源管理器完全关闭 → 出现新窗口/新标签，目标高亮，窗口在前台。
   - 资源管理器开着、目标目录未在任何标签 → 新增该目录的标签并选中。
   - 目标目录就是**当前标签** → 只选中并置前，不新增标签、不刷新。
   - 目标目录在**后台标签** → 你正在看的标签保留，目标标签被带到前台并选中；
     实测这一条会为该目录**多出一个标签**。不想多标签就设
     `$env:REVEAL_TAB_POLICY='overwrite'`（改为覆盖当前标签，标签数不增长）。
6. 配置项负例：`$env:REVEAL_TAB_POLICY='sideways'` 后 reveal → 退出码 2，
   stderr 提示 `REVEAL_TAB_POLICY must be 'newtab' or 'overwrite', got 'sideways'`；
   改回 `newtab`/`overwrite` 或不设该变量即恢复正常。
   同理 `$env:REVEAL_DRY_RUN='maybe'` → 退出码 2，stderr 提示
   `REVEAL_DRY_RUN must be '1'/'0', 'true'/'false' or 'yes'/'no', got 'maybe'`；
   此时 `reveal.exe --help` 仍然返回 0——读说明书不该先要求一个正确的配置值。
7. 其它负例与自检：`reveal.exe`（2，usage）、`reveal.exe a b`（2）、`reveal.exe ""`（2）、
   `reveal.exe C:\nope.txt`（2，stderr 含该路径）；
   `reveal.exe --help` 与 `reveal.exe -h` → 0，整段用法在 **stdout**、stderr 为空；
   `reveal.exe --helpme` → 2 且按"路径不存在"处理，`reveal.exe 目标文件 --help` → 2 且按
   "参数个数 2"处理（help 只认第一个参数）。
   `$env:REVEAL_DRY_RUN='0'`（或 `false`/`no`/空串）等于没设：再跑第 4 步就是真实弹窗，
   而不是 `would reveal:`。
8. 若第 4 步只打开目录而未高亮：说明补偿没能在 Shell COM 的窗口列表里匹配到该视图
   （例如该 shell 不暴露 `FolderView`）。判别法：对同一窗口用 PowerShell 的 Shell COM
   `FolderView.SelectItem` 手动选中——若手动可见高亮而本工具没有，则补偿路径在该 shell 上失效，
   请记录 Windows 版本；若手动也不高亮，则是本机 shell 缺陷（build 26100 上
   `SHOpenFolderAndSelectItems` 与 `explorer /select,` 单独使用时即如此，本工具靠补偿绕开）。
9. 路径形态归一化（用 dry-run，**不开窗口**）。注意 `\\?\` 这种串经 git-bash/PowerShell 传参
   常被削掉一个反斜杠，所以用 `[char]92` 拼出来再传，别直接打字符串：

   ```powershell
   $b = [char]92; $env:REVEAL_DRY_RUN = '1'
   & '.\target\release\reveal.exe' ($b+$b+'?'+$b+'C:\Windows\notepad.exe'); "exit=$LASTEXITCODE"
   # would reveal: C:\Windows\notepad.exe          exit=0
   & '.\target\release\reveal.exe' ($b+$b+'?'+$b+'UNC\fileserver\share\a.txt'); "exit=$LASTEXITCODE"
   # reveal: path does not exist: \\fileserver\share\a.txt   exit=2   ← 报的是还原后的名字
   & '.\target\release\reveal.exe' 'C:Windows\notepad.exe'; "exit=$LASTEXITCODE"
   # would reveal: C:\Windows\notepad.exe          exit=0   ← 盘相对已锚到盘根
   $env:REVEAL_DRY_RUN = '0'
   ```

   第 2 条在没有该共享时才会走到"不存在"；若 `\\fileserver\share\a.txt` 在你机上真实存在，
   它会以 `exit=0` 打印该 UNC 路径，同样是预期结果。整段都开着 dry-run，所以**一个窗口都不会开**；
   最后一行把开关设回 `'0'`（`$null` 是删除变量，同样算关），要看真实弹窗就在此后再跑一次第 3 步。

## 许可与仓库内容

- **许可**：MIT，全文见 [LICENSE-MIT](LICENSE-MIT)，与 `Cargo.toml` 的 `license = "MIT"` 一致；
  版权抬头 `Copyright (c) 2026 seeyuer`。选它而不是双许可：这个项目只有一个 exe 和一段 FFI 声明，
  没有需要专利条款来保护的实现细节，最短的许可证就够用。
- **仓库**：<https://github.com/jinghu-moon/win-reveal>
- **不入库的三样东西**（[.gitignore](.gitignore) 里各带一条为什么）：
  `target/` 是 497 MB 可再生产物；`rust_sources/` 是调研用的**第三方源码**，把它提交进一个可发布
  的仓库等于由我们来承担再分发义务——来源记在 `download-rust-repos.ps1`（它抓 GitHub 上的
  `showfile`/`win-desktop-utils`/`ublx` 三个），另外两个 crates.io 的包按上面调研表的版本号手动取；
  `tmp/` 是手动验证第 2 步在项目内造的中文+逗号+空格夹具。
- `Cargo.lock` **入库**：这个项目最终交付的是一个要被人复制走的 exe，锁定 `windows 0.62.2`
  的精确解析结果才谈得上"官方形态可复现"。

