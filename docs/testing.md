# 测试

## 怎么跑

```powershell
cargo test                                          # 默认形态：54 passed / 3 ignored / 3 doctests
cargo test --features official-bindings             # 官方绑定形态：同一基线
cargo clippy --all-targets                          # 零警告（含 --features official-bindings）
cargo +1.88.0 check --all-targets                   # MSRV 实测
REVEAL_UI_TEST=1 cargo test --test ui_reveal -- --ignored --nocapture   # 真弹窗，会开标签
REVEAL_LATENCY_TEST=1 cargo test --test latency -- --ignored --nocapture # 真弹窗，测时延
```

## 不弹窗的端到端

- 可注入边界：`Revealer` trait；`RecordingRevealer` 记录被要求选中的路径；二进制的
  `REVEAL_DRY_RUN=1|true|yes` 换成打印型实现 → 端到端测试**不会弹出资源管理器**。
- `src/lib.rs` 单元测试 25 个：参数个数/空参/缺失、相对路径按 cwd 解析、空格+中文、逗号+`&`、
  目录输入、尾分隔符、`..`、盘根拒绝、shell 失败映射为退出码 1、`REVEAL_TAB_POLICY` 默认值/
  大小写/非法值、`REVEAL_DRY_RUN` 的开/关/非法三档映射、每条 usage 错误以 `USAGE` 常量收尾
  （`--help` 里嵌的是同一个常量）、`Error` 确实实现了 `std::error::Error`（否则宿主项目的 `?`
  编译不过，而这没有别处守着），以及四种路径形态（`\\?\C:` 改写后仍走完整 reveal、
  `\\?\UNC\` 还原成 UNC、盘相对 `C:a.txt` 锚到盘根、设备命名空间形态原样不动）；加上库宿主专属的
  两条——本线程已被宿主按 MTA 初始化时 `init_apartment` 必须回报"套间不是我们的"而不是失败
  （`RPC_E_CHANGED_MODE` 是负 HRESULT，COM 却照常可用；这条真的调 `CoInitializeEx`，但不碰
  shell，所以不开窗），以及两个 revealer 都满足 `Send + Sync + Revealer`（recorder 里换成
  `RefCell` 就让宿主的 `Arc<dyn Revealer + Send + Sync>` 编译不过）；外加
  `src/automation.rs` 中路径匹配与标题可见性（前缀边界、中文、空名）6 个纯函数测试。
- doctest 3 条（`cargo test --doc`）：crate 级与 `reveal_with` 两条标 `no_run`——它们调用真实
  shell，编译校验足够，执行就会在开发机上弹资源管理器窗口；`run` 一条用 `RecordingRevealer`
  真跑，断言退出码、记录到的路径与写进 `stderr` 缓冲的文案。README 里教的那行
  `[dependencies]` 因此有守卫。
- `tests/cli.rs` 19 个：真实二进制的退出码与 stderr 文案（全部在触达 shell 之前失败，或只失败在
  读取标签策略那一步），`--help`/`-h` 走 stdout + 退出 0 且与错误路径共用同一行 usage，
  `--helpme` 与出现在第二个位置的 `--help` 仍按普通参数处理，`REVEAL_DRY_RUN` 的 `0|false|NO|空` 确实关到了
  "会去读标签策略"的程度而 `1|true|YES` 确实跳过了整个 shell 分支，以及 dry-run 下的精确路径回显。
  每个用例都先 `env_remove` 掉两个配置项，避免跑 cargo 的那个 shell 里残留的变量改变观察结果。

## 交叉校验（`src/crosscheck.rs`）

4 个，只在 `cargo test` 编译，二进制不依赖它；调的都是**不会创建窗口**的参数与错误路径
（`SHOpenFolderAndSelectItems` 只喂空 folder——实测 build 26100 上在开窗前就 `E_INVALIDARG`，
调用前后 `Shell.Windows` 计数不变），因此不开窗口：

1. 同一个 `\\?\…` 路径，官方 `Shell::SHParseDisplayName` 与我们的声明返回**同一个**
   `0x80070057`，改写后两边都 `S_OK` → 归一化补的是 API 的约束，不是我们自己声明的错。
2. 官方解析出一个 PIDL 后 `ILClone` 一份，用**我们的** `ILFindLastID`/`ILRemoveLastID`/`ILFree`
   操作它，再用官方 `ILIsParent(ours, full, true|false)` 断言截留的正是直接父级，并比较子项字节
   → 拆分算法与官方 helper 一致，且 `ILFree` 与官方共用同一个分配器。
3. 空 folder 的 `SHOpenFolderAndSelectItems`：两套绑定都返回 `0x80070057` → 把 reveal 真正调用
   的那个四参数签名（`pidl_folder, cidl, apidl, dw_flags`）与官方包装里的 `Option`/切片重建钉住；
   这是以前没人守的一条，因为真 folder 参数会弹资源管理器。
4. `ILFree(空 PIDL)`：两套绑定都当 no-op 不崩 → 解析失败时我们的代码照样把空 PIDL 交给 `ILFree`，
   这是活路径。

交叉校验始终走 `ffi::native`（不是当前形态选中的那套），否则 `--features official-bindings` 下两边
变成同一份代码，断言就成了自证。实测还留下一条 PIDL 常识：**同一路径两次解析的字节并不相同**
（文件夹 item id 内嵌 shell 会刷新的时间戳），所以比较只能在同一块内存上做。

## 真弹窗测试（`tests/ui_reveal.rs`）

2 个 `#[ignore]` opt-in 测试，`REVEAL_UI_TEST=1` 时真实弹窗。

第一个三段断言：
① 退出码 0 **且** UIA 读到 `IsSelected=True` **且**没有以该文件名为标题的 `Edit`（没进重命名态）；
② 同目录第二次 reveal，断言视图数 ≥1 且不变（视图数断言前先校验非 0，否则"没多开窗口"会因为
查询本身失效而虚假通过）；③ 把承载夹具的窗口**最小化**后第三次 reveal，断言窗口不再最小化
且文件高亮——这一步专门守着"窗口看不见时选中不可见"这一类回归。

第二个守 `overwrite`：先用默认策略开两个夹具目录（target 在先、foreground 在后），断言标题栏上是
foreground 而不是 target——本机若没把两者并进同一个窗口，就没有"可覆盖的可见标签"这回事，测试
直接红而不是假装通过；再 `overwrite` reveal target 里的第二个文件，断言退出 0 + UIA 高亮 +
无重命名态，且**总标签数不变**、target 标签 +1、foreground 标签 −1、标题栏换成 target 且不再是
foreground。注意 UIA 那半边只证明"选中落在某个标签上"：26100 实测后台标签的 ListItem 同样留在
UIA 树里（两个同目录标签都报 `IsSelected=True`），所以"用户看得见"只能靠标题栏前缀那条判断，
也正是工具自己唯一的可见标签信号。

两个测试共用一把进程内 `Mutex`：cargo 会在同一个测试二进制里并行跑它们，而它们读同一个桌面、
同一个标签窗口的标题栏，并行会互相改掉对方的标题（首次真机跑就是这样废掉一条断言的）。
收尾 `WM_CLOSE` 只关那些标签全部属于夹具的窗口（共用 HWND 的窗口里有用户自己的标签）。
该测试会在当前资源管理器窗口里留下标签（shell 会把新目录并入已有窗口），跑完请手动 `Ctrl+W`。

`tests/latency.rs` 是第三个 opt-in 入口，见 [latency.md](latency.md)。

## 基线

仓库原本没有 Cargo 工程（`cargo test` → `could not find Cargo.toml`），无既有功能可回归。
最近一次全量复验（2026-10-27，绑定解耦那批改动之后：`native` 模块改成
`#[cfg(any(not(feature = "official-bindings"), test))]`、crosscheck 加两条契约测试（空 folder 的
`SHOpenFolderAndSelectItems`、`ILFree(空 PIDL)`），单测数 33→35、默认总数 52→54；两种形态各跑一遍，
结果相同）：
`cargo test` 与 `cargo test --features official-bindings` → 54 passed, 0 failed, 3 ignored，
另有 doctest 3 passed；`cargo clippy --all-targets`（含 `--features official-bindings`）零警告；
`cargo +1.88.0 check --all-targets` 与 `cargo +1.88.0 test --doc` 通过（MSRV 实测）；
`REVEAL_UI_TEST=1 cargo test --test ui_reveal -- --ignored --test-threads=1` 在两种形态下各
→ 2 passed（本批真跑，其中 `overwrite` 一档仍实测到总标签数不变、target +1、foreground −1、
标题栏换成目标目录）。时延数字沿用同日库宿主适配那批的实测（见下段）——本批只删死声明、加测试，
不碰被计时的 reveal 路径，故未重跑 `latency`（省 8 个标签）；新数字见 [latency.md](latency.md)。

同一次绑定解耦复验的产物实测：门控 `native` 前后，两种形态的 release exe **都仍是 297,984 B**
（哈希因注释导致的行号位移而变）→ 删掉的是官方形态里的死 extern 声明，链接器本就不码化它们，
收益是意图与少编译一个模块，不是体积。crosscheck 新增两条都不开窗（空 folder 前后 `Shell.Windows`
计数实测不变，`ILFree(空)` 无崩溃）。

更早一批（库宿主适配：`init_apartment` 三态、`RecordingRevealer` 换 `Mutex`、三段等待改退避）
复验为 52 passed；

同日发布元数据收口之后（`Cargo.toml` 加 `authors`/`readme`/`exclude`、新增 `CHANGELOG.md`），
发布前校验：`cargo package --list` 打出的 23 个文件含 `CHANGELOG.md`、不含被 `[package] exclude`
挡下的 `download-rust-repos.ps1`；`cargo publish --dry-run` 在隔离目录里重新编译通过
（本批改动源码后为 181.8 KiB / 压缩 59.8 KiB；这个字节数随注释文本浮动，以现测为准）。`.gitignore` 补了一条 `/.spec-workflow`——发现
`cargo package` 会把未忽略的未跟踪文件（这里是某外部工具生成的 7 个模板）一并打进 crate，挡掉后
清单回到干净的 23 个。非 Windows 目标的报错面用 `cargo check --target x86_64-linux-android` 量过：
9 条错误，首条即 crate 里那句 `compile_error!`。
