# win-reveal

在 Windows 资源管理器里打开一个文件（或目录）**所在的目录，并把它高亮选中**——不是只打开目录。

```powershell
reveal.exe "C:\some dir\文件.txt"
```

- 仅 Windows（非 Windows 目标直接 `compile_error!`），不做跨平台抽象。
- 默认构建零第三方依赖：`cargo tree` 里没有 normal 依赖，只链接系统的 `shell32`/`ole32`。
- 路径走 shell 命名空间（UTF-16 PIDL）而不是拼命令行，空格、中文、逗号、`&` 都不构成解析风险。
- 结果是"选中"，不是"进入重命名框"；窗口最小化时也会恢复并置前。
- 同一个 crate 也是库：错误类型实现了 `std::error::Error`，可直接落进调用方的 `?` 链路。

## 安装

```powershell
cargo build --release                          # → target\release\reveal.exe
cargo build --release --features official-bindings   # 换官方绑定形态，行为完全相同
```

在 `Cargo.toml` 里被依赖。这个 crate 在非 Windows 目标直接 `compile_error!`（没有跨平台桩），
所以跨平台宿主要把依赖挂在 cfg 下，否则在 Linux/macOS 上连 `cargo check` 都过不去——用
`--target x86_64-linux-android` 实测：9 条错误，第一条是那句 `compile_error!`，其余来自
`std::os::windows` 与 UTF-16 转换，属于"这里根本没有实现"的正常表现：

```toml
[target.'cfg(windows)'.dependencies]
win-reveal = "0.1"          # v0.1.0 还没发布到 crates.io；在那之前写成 path = "../win-reveal"
```

包名带 `win-` 限定（crates.io 上 `reveal` 已被一个图片查看器占用，`windows-*` 前缀实际属于微软
windows-rs 家族），而**命令仍然是 `reveal`**：那是人要敲的东西，平台限定由包名承担。

## 命令行

```text
usage: reveal <file-or-folder-path>
```

恰好一个参数：路径不存在、无父目录（盘根）、参数个数不对、空参数都会退出 2 并在 stderr 给出原因。
`--help` 或 `-h` **只认作第一个参数**，把用法、两个配置项与退出码打到 stdout 后退出 0；
`--helpme` 仍按路径处理，`reveal.exe 那个文件 --help` 算"参数个数 2"。

| 退出码 | 含义 |
|:--|:--|
| 0 | 已成功把"打开目录并选中"的请求交给 shell（`--help` 也走这一档） |
| 1 | shell 调用本身失败，stderr 给出失败的 Win32 步骤与 HRESULT |
| 2 | 输入错误：参数个数/空参数/路径不存在/无父目录/配置项取值不认识 |

成功判据取 `SHOpenFolderAndSelectItems` 的 HRESULT，**不读 `explorer.exe` 的退出码**——本机实测
它成功时也返回 1（[为什么这么选](docs/research.md)）。

### 配置项

两个都用环境变量，取值不区分大小写；**写法不认识就退出 2**，不静默按默认走。

| 变量 | 取值 | 作用 |
|:--|:--|:--|
| `REVEAL_TAB_POLICY` | `newtab`（默认）/ `overwrite` | 目标目录已开着但在你**没在看**的标签里时怎么办，见下 |
| `REVEAL_DRY_RUN` | `1`/`true`/`yes` 开，`0`/`false`/`no`/空 关 | 完全不碰 shell，只在 stdout 打印 `would reveal: <绝对路径>`；端到端测试与排查用 |

### 目标目录已经在后台标签里时

这是唯一需要你决定的局面（其余两种起始状态下两个取值表现一致）：

| 起始状态 | `newtab`（默认） | `overwrite` |
|:--|:--|:--|
| 目标目录就是当前可见标签 | 只选中并置前 | 同左 |
| 目标目录没在任何标签开过 | shell 开一个新标签并选中 | 同左 |
| 目标目录在**后台标签** | 你正在看的标签不动，把目标标签带到前台并选中；实测代价是为该目录**多出一个标签**，重复 reveal 会累积 | 把你正在看的标签**导航**到目标目录：标签数不增长，代价是那个标签原来的内容被替换 |

两种代价都由真机用例守着（[docs/testing.md](docs/testing.md)）。
库宿主不必为了选策略去改**自己进程**的环境变量：`ShellRevealer::reveal_with(path, Switch::Overwrite)`
把同一个选择作为参数接收。

## 作为库使用

```rust
use win_reveal::{Revealer, ShellRevealer, Switch};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = std::env::current_exe()?;      // 任何存在且有父目录的路径
    ShellRevealer.reveal(&target)?;             // 标签策略来自 REVEAL_TAB_POLICY
    ShellRevealer::reveal_with(&target, Switch::Overwrite)?;  // 或者当参数给
    Ok(())
}
```

这段是编译过的：`src/lib.rs` 有 3 条 doctest，碰真实 shell 的两条标了 `no_run`（只编译，
绝不替 `cargo test` 弹资源管理器）。公开面只有这些，其余（`automation`、`ffi`、`normalize`、
标签切换的实现）都是模块私有的：

| 项 | 位置 | 给宿主项目做什么 |
|:--|:--|:--|
| `trait Revealer` / `ShellRevealer` / `RecordingRevealer` | [src/lib.rs:130](src/lib.rs#L130) / [:147](src/lib.rs#L147) / [:315](src/lib.rs#L315) | 唯一的注入边界；recorder 让宿主自己的测试不必弹资源管理器 |
| `ShellRevealer::reveal_with(path, Switch)` | [src/lib.rs:163](src/lib.rs#L163) | 与 `reveal()` 同一条路，只是策略来自参数。宿主要行为可预测就用它：`reveal()` 的策略来自进程环境变量 `REVEAL_TAB_POLICY`，那是个看不见的输入，而 `set_var` 在新 edition 里已是 unsafe、还会被子进程继承 |
| `enum Switch { NewTab, Overwrite }` | [src/automation.rs:572](src/automation.rs#L572)，由 [src/lib.rs:49](src/lib.rs#L49) 再导出 | 就是上面那张代价表的两列；`NewTab` 也是 `REVEAL_TAB_POLICY` 的默认值 |
| `enum Error`（7 个变体） | [src/lib.rs:78](src/lib.rs#L78) | `NotFound` / `NoParent` / `BadSetting` / `Shell{step, hresult}` 可分派，`Display` 文案与 CLI 一致 |
| `resolve(raw, cwd)` | [src/lib.rs:384](src/lib.rs#L384) | 只做形态归一＋绝对化＋存在性与父目录检查，完全不碰 shell |
| `run(args, cwd, revealer, stderr) -> i32` | [src/lib.rs:432](src/lib.rs#L432) | 整套 CLI 语义，但 cwd、stderr、实现都是参数，可嵌进 GUI 或自己的命令行 |
| `EXIT_OK`/`EXIT_SHELL`/`EXIT_USAGE`、`USAGE`、`DRY_RUN_VAR`、`dry_run_setting()` | [src/lib.rs:52-74](src/lib.rs#L52)、[:227](src/lib.rs#L227) | 包装 `run` 时不必抄数字；复用同一行 usage 与同一套取值规则 |

**库宿主还需要知道三件事**（都不必额外配置）：

- **套间**：`reveal_with` 在调用线程上初始化 COM，但**沿用宿主已有的套间**——包括线程已经按
  MTA 初始化的情况。那时 `CoInitializeEx` 返回 `RPC_E_CHANGED_MODE`（负 HRESULT，COM 却完全
  可用），我们照常执行，并且**不会**替宿主 `CoUninitialize`；只有其它真正的失败才映射成
  `Error::Shell`。
- **阻塞**：等 shell 刷新用的是 `thread::sleep`，不泵消息。成功路径一般在几十毫秒内返回，
  最坏 3 s 上限。GUI 宿主若不想让"在资源管理器中显示"卡住 UI 线程，把这个调用放到工作线程。
- **线程**：`ShellRevealer` 与 `RecordingRevealer` 都满足 `Send + Sync`，可以放进
  `Arc<dyn Revealer + Send + Sync>` 交给别的线程（recorder 内部是 `Mutex`，不是 `RefCell`）。

**MSRV `1.88`**：这是实测过的最低工具链，本机装了 1.88.0，`cargo +1.88.0 check --all-targets`
（两种形态）与 `cargo +1.88.0 test --doc` 都通过。更低的版本没装、没实测，就不写进元数据。

## 两种构建形态，同一套算法

差别只在 shell 绑定的来源（见 [src/ffi.rs](src/ffi.rs)）：

| 形态 | 命令 | shell 绑定 | 产物（本次构建实测） |
|:--|:--|:--|:--|
| 零依赖（默认） | `cargo build --release` | `extern "system"` 自声明 7 个函数 | `reveal.exe` 297,984 B（留档名 `reveal-native.exe`） |
| 官方绑定 | `cargo build --release --features official-bindings` | 同名包装，转调 `windows` 0.62.2 | 297,984 B（`reveal-official.exe`） |

官方形态同样不引入运行时第三方代码：调的还是那两个系统 DLL，只是绑定由微软生成，编译期多出
`windows-core` 等 crate。这一档字节差不是恒定的：早先测到官方形态大 1,024 B，这次两者同为
297,984 B（哈希不同，确认是两份不同产物），所以它只是链接布局的偶然，不是形态的固有代价。
两者的可观察行为、退出码与时延同一量级，按需要选一种即可。feature 不叫 `windows`：在一个整体
仅 Windows 的 crate 上，`--features windows` 读起来像"默认构建还不支持 Windows"。

## 测试

```powershell
cargo test                                        # 54 passed / 0 failed / 3 ignored + 3 doctests
cargo test --features official-bindings           # 同一基线
cargo clippy --all-targets                        # 零警告
REVEAL_UI_TEST=1 cargo test --test ui_reveal -- --ignored          # 真弹窗断言高亮
```

默认测试**不会弹出资源管理器**：`Revealer` 是可注入边界，二进制侧用 `REVEAL_DRY_RUN=1` 走打印型
实现。各测试目标守什么、以及那 3 个 `#[ignore]` 真弹窗用例的断言与环境代价，见
[docs/testing.md](docs/testing.md)。

## 表现预期

- **时延**：地板约 18 ms；在你正看着的文件夹里换选一个文件约 50–60 ms；目标目录没开过约 0.9 s、
  条目刚写入约 0.4–1.2 s、后台标签配默认策略约 1.1–1.3 s。秒级那部分花在**等 shell**（有墙钟
  上限的退避轮询），不花在调用上；它随你打开的标签数线性增长。两种形态同数量级，数据与测法见
  [docs/latency.md](docs/latency.md)。
- **只打开目录、没有高亮**：多半是该 shell 不暴露 `FolderView`，或本机 shell 缺陷。判别方法与
  两种结果分别意味着什么见 [docs/manual-verification.md](docs/manual-verification.md) 第 8 步。
- **路径形态**：`\\?\C:\…`、`\\?\UNC\…`、盘相对 `C:a.txt` 会被归一化，设备命名空间形态原样交给
  shell。为什么必须改写：[docs/how-it-works.md](docs/how-it-works.md)。

## 深入阅读

| 文档 | 内容 |
|:--|:--|
| [docs/how-it-works.md](docs/how-it-works.md) | 选型的第一性原理、PIDL 拆分、FFI 面、标签页四个实测事实、`SVSI_EDIT` 重命名坑、手写 IDispatch/BSTR 的约束 |
| [docs/research.md](docs/research.md) | 五个同类项目的源码对比（带 file:line）与本机 build 26100 的实测清单 |
| [docs/testing.md](docs/testing.md) | 每个测试目标守住什么、交叉校验、真弹窗测试的序列化与残留、最近一次基线 |
| [docs/latency.md](docs/latency.md) | A–F 六档时延数据与读法 |
| [docs/manual-verification.md](docs/manual-verification.md) | 9 步人工复核清单（真实弹窗与选中） |
| [CHANGELOG.md](CHANGELOG.md) | 0.1.0 的公开面清单、已知限制，以及 0.x 的版本策略 |

## 许可与仓库内容

MIT，全文见 [LICENSE-MIT](LICENSE-MIT)，版权抬头 `Copyright (c) 2026 seeyuer`；
仓库 <https://github.com/jinghu-moon/win-reveal>。`[package] authors` 同样署 `seeyuer`，
不放邮箱——这行字会原样出现在 crates.io 上并被索引。

`.gitignore` 排除四样东西，各带一条理由：`target/`（497 MB 可再生产物）、
`rust_sources/`（调研用的**第三方源码**，提交进可发布的仓库等于由我们承担再分发义务；来源记在
`download-rust-repos.ps1`）、`tmp/`（人工验证的中文+逗号+空格夹具）、`.spec-workflow/`
（外部工具在项目根生成的脚手架）。最后一条是发布前才补的：`cargo package` 会把**未被忽略的未跟踪
文件**一起打进 crate，所以漏掉它等于把无关工具的模板发到 crates.io——它现在被挡住，dry-run 的
23 文件清单里不再有它。
`Cargo.lock` **入库**：交付的是一个要被人复制走的 exe，锁定 `windows 0.62.2` 的精确解析结果才谈得上
"官方形态可复现"。`download-rust-repos.ps1` 入库但用 `[package] exclude` 挡在 crate 外：
打包环境里没有 `rust_sources/` 可写，那个脚本进 crate 就是一件不能用的东西。

分类取自 crates.io 的分类表并已核实：`os::windows-apis`（库）+ `command-line-utilities`（`reveal`
这个二进制）。文档站在本地是 `cargo doc --no-deps`；docs.rs 那侧因为 crate 在非 Windows 目标
`compile_error!`，那个站点的构建机上编译不过，所以**不要把 docs.rs 链接当"文档在这里"用**——
读 README 与 `docs/`。

发布前校验已过：`cargo publish --dry-run` 打包 23 个文件、181.8 KiB（压缩 59.8 KiB）并在隔离目录里
重新编译校验，最后一行 `warning: aborting upload due to dry run` 是它的正常收尾。
`main` 已推到 <https://github.com/jinghu-moon/win-reveal>；`v0.1.0` 是**本地附注标签**，打在已推送的
这棵树（HEAD）上。剩下推这个标签、以及真正的 `cargo publish`（需要 crates.io token）还是人手上的
动作，尚未执行。
