# 实现原理

本文记录 `win-reveal` 的选型依据与 build 26100 上的实测约束。总览见 [../README.md](../README.md)。

## 第一性原理 + 奥卡姆剃刀

不可再还原的事实只有三条：

1. Windows 上"打开目录并选中条目"的**文档化**入口只有一个：`SHOpenFolderAndSelectItems`，
   它返回 HRESULT，因此失败是可判定的。
2. `explorer.exe /select,<path>` 是未文档化的命令行约定，其退出码无意义（本机实测成功也返回 1），
   且路径要经过一次命令行拼接，空格/逗号/引号都是真实的解析风险。
3. 路径一旦走 shell 命名空间（UTF-16 PIDL），就完全不经过命令行，转义问题从根源上消失。

由这三条直接推出：调用 `SHOpenFolderAndSelectItems`，路径以 UTF-16 传入，用 HRESULT 决定退出码。
其余一切（CLI 框架、跨平台层、canonicalize、命令行拼接）都是不必要的实体，全部不引入。
唯一保留的一次输入改写是路径形态归一化——它不是抽象层，而是下面那条实测约束的直接结果。

## 路径形态归一化（`normalize`，[src/lib.rs:323](../src/lib.rs#L323)）

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
`\\?\` 与 `\\?\UNC\` 两种形态由官方 `windows` 绑定在同一进程里交叉验证过（见
[testing.md](testing.md)），确认拒绝来自 API 而不是我们的声明。

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

## 选中、标签页与置前（`src/automation.rs`）

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
   在**新建**窗口时丢弃它的选中请求（见 [research.md](research.md)），因此打开后再走第 1 步
   同一条路补选中并置前，最多重试 15×200ms 等待窗口创建完成。

标签页的四个实测事实决定了上面的形状：

- 一个带 N 个标签的窗口在 `Shell.Windows` 里是 **N 条记录**（共用同一 HWND），且没有任何字段
  标明哪条是当前标签。可用信号只有标题栏：`<当前标签显示名> 和 N 个其他选项卡 - 文件资源管理器`
  （开启"在标题栏显示完整路径"时前半段是完整路径）。判定用"标题以该标签的名字**开头且随后是
  空格**"，不能用"包含"：`D:\…\p1` 会被 `D:\…\p1-keeper 和 …` 的标题命中，于是把一个后台标签
  当成可见标签，选中打在看不见的地方——用户读作"没反应"。
- 对**后台标签**调 `FolderView.SelectItem` 会真的选中，但用户看不见；`Navigate2` 打在后台标签上
  只刷新它、不切过去，`SHOpenFolderAndSelectItems` 也不切。UIA 也帮不上：标签项虽然能读到
  `IsSelected`，却没有 Invoke/SelectionItem 模式，既不能切换也不能关闭。能切换的只有
  `explorer.exe <目录>`（会多一个标签）与覆盖当前标签的 `Navigate2`，这就是
  `REVEAL_TAB_POLICY` 的由来。
- 后台标签的 `Document` 依然可用，所以判定用 `GetWindowTextW`，句柄取自
  `IWebBrowser2.HWND`：该属性以 `VT_UI8`（type code `0x14`）返回，句柄就在变体载荷里，
  用 `IsWindow` 校验后采用——按 `VT_I4` 读会全部拿到 0，这也是早先置前与可见性判断失灵的原因。
  另外 `Shell.Windows` 在无窗口时仍含"此电脑"等非 `CabinetWClass` 条目，故只接受
  `CabinetWClass` 的句柄，避免桌面/此电脑条目把 reveal 吞掉。
- 窗口**最小化**时标题仍显示它的当前标签（所以"可见"判定为真），但 UIA 里列不出任何条目，
  选中成功了用户也看不到——因此每次成功路径都要 `bring_to_front`，它负责 `SW_RESTORE` 与抢前台。

## 选中的状态位

`SelectItem` 的状态位取 `SVSI_SELECT | SVSI_DESELECTOTHERS | SVSI_ENSUREVISIBLE | SVSI_FOCUSED`
（`0x1|0x4|0x8|0x10`）。这里有个真实的坑：`_SVSIF` 里 `SVSI_EDIT = 0x3` **包含** `SVSI_SELECT`，
所以把 bit0+bit1 一起置上（早期版本用的 `1|2`，以及看似无害的 `7`）等于请求"选中并进入重命名"，
用户症状就是 reveal 后文件直接进了重命名框。位值以 `ShObjIdl_core.idl` 为准。

## 手写 IDispatch 客户端

该客户端是手写的后期绑定 IDispatch（`GetIDsOfNames`/`Invoke` + 24 字节 `VARIANT`）；
两种构建形态都保留它——`official-bindings` feature 只换掉那 7 个 shell 声明，不换这里，因为
VARIANT/`DISPPARAMS` 的构造本身就是要测的代码，交给生成绑定等于把被测对象换成了依赖库。
作为参数传出的 `BSTR` 必须是 `SysAllocString` 自己拥有的：
传借用指针会让变体析构里的 `VariantClear` 释放不属于我们的内存，进程以 `0xC0000374`
（堆损坏）崩掉。补偿失败不改变退出码：文档化 shell 调用已成功、目录已打开，补偿只负责把本机
shell 丢掉的那一次选中补回来。
