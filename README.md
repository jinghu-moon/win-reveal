# win-reveal

`win-reveal` 是一个仅支持 Windows 的 Rust 工具和库。它在资源管理器中打开文件或目录所在的文件夹，并高亮选中目标。

```powershell
reveal "C:\work\报告 2026\结果, final.txt"
```

路径通过 Windows shell 命名空间以 UTF-16 传递，不经过 `explorer.exe` 命令行拼接，因此空格、中文、逗号和 `&` 不需要额外转义。

## 安装

从源码构建命令行程序：

```powershell
cargo build --release
```

生成的程序位于 `target\release\reveal.exe`。

发布到 crates.io 后，作为库依赖时：

```toml
[target.'cfg(windows)'.dependencies]
win-reveal = "0.1"
```

这个 crate 没有跨平台实现。跨平台宿主应使用 Windows 条件依赖，否则非 Windows 构建会被明确拒绝。

## 命令行

```text
reveal <file-or-folder-path>
```

参数必须恰好是一个存在的文件或目录。相对路径按当前工作目录解析；盘根目录因没有可打开的父目录而被拒绝。

```powershell
reveal --help
reveal -h
```

`--help` 和 `-h` 只有作为第一个参数时才表示帮助选项。

退出码：

| 代码 | 含义 |
| ---: | --- |
| `0` | 请求已成功交给 Windows shell；帮助也使用此代码 |
| `1` | shell 调用失败，错误信息包含失败步骤和 HRESULT |
| `2` | 参数、路径或环境变量配置无效 |

### 环境变量

`REVEAL_DRY_RUN` 可以在不打开资源管理器的情况下检查路径解析：

```powershell
$env:REVEAL_DRY_RUN = "1"
reveal ".\报告 2026\结果, final.txt"
# would reveal: C:\...\报告 2026\结果, final.txt
```

接受 `1`、`true`、`yes` 表示开启；`0`、`false`、`no` 或空值表示关闭。其他值会以退出码 `2` 拒绝。

`REVEAL_TAB_POLICY` 控制目标目录已经存在于后台 Explorer 标签时的行为：

| 值 | 行为 |
| --- | --- |
| `newtab` | 默认值。保留当前可见标签，把目标标签带到前台；可能产生一个重复标签 |
| `overwrite` | 导航当前可见标签到目标目录；标签数量不增加，但当前标签内容会被替换 |

取值不区分大小写。当前标签就是目标目录，或目标目录尚未打开时，两种策略行为相同：复用或打开目标目录并选中条目。

## 作为库使用

```rust,no_run
use win_reveal::{Revealer, ShellRevealer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target = std::env::current_exe()?;
    ShellRevealer.reveal(&target)?;
    Ok(())
}
```

如果不希望从进程环境读取标签策略，可以直接传入策略：

```rust,no_run
use win_reveal::{ShellRevealer, Switch};

let target = std::env::current_exe()?;
ShellRevealer::reveal_with(&target, Switch::Overwrite)?;
```

主要公开 API：

| API | 用途 |
| --- | --- |
| `Revealer` | shell 实现与测试替身之间的注入边界 |
| `ShellRevealer` | 通过 Windows Explorer 执行 reveal |
| `ShellRevealer::reveal_with` | 显式传入 `Switch`，不读取标签策略环境变量 |
| `RecordingRevealer` | 记录请求，供宿主测试使用，不打开窗口 |
| `Switch` | `NewTab` 或 `Overwrite` |
| `Error` | 路径、配置和 Win32 shell 错误，支持 `std::error::Error` |
| `resolve` | 按指定工作目录解析、归一化并验证路径 |
| `run` | 复用完整 CLI 语义，同时注入参数、revealer 和错误输出 |

`ShellRevealer` 会在调用线程使用 COM，并沿用宿主已经建立的 COM 套间。调用会等待 Explorer 刷新，通常几十毫秒完成，冷路径或后台标签场景可能需要约一秒。视图查找和打开后的补选分别最多等待 2 秒和 3 秒；少见的连续等待情况下总阻塞时间可能接近 5 秒。GUI 程序应在工作线程调用它。

`RecordingRevealer` 和 `ShellRevealer` 都满足 `Send + Sync`。库调用仍要求目标路径存在且有父目录。

## 构建形态

默认构建使用手写 Win32 FFI，不引入第三方运行依赖：

```powershell
cargo build --release
```

如果宿主已经使用微软的 `windows` crate，可以选择官方绑定：

```powershell
cargo build --release --features official-bindings
```

两种形态共享同一套路径处理、PIDL、Explorer 标签和错误逻辑；feature 只替换 7 个 shell/COM 入口的绑定来源。两者都调用 Windows 系统 DLL，官方绑定不会改变运行时行为。

## 测试与验证

常规测试不打开资源管理器：

```powershell
cargo test
cargo test --features official-bindings
cargo clippy --all-targets
cargo doc --no-deps
```

真实 Explorer 测试需要显式启用，会创建和切换标签：

```powershell
$env:REVEAL_UI_TEST = "1"
cargo test --test ui_reveal -- --ignored --test-threads=1 --nocapture
```

完整测试边界、时延数据和人工复核步骤见 [docs/testing.md](docs/testing.md)、[docs/latency.md](docs/latency.md) 和 [docs/manual-verification.md](docs/manual-verification.md)。实现选择见 [docs/how-it-works.md](docs/how-it-works.md)，调研记录见 [docs/research.md](docs/research.md)。

## 已知限制

- 仅支持 Windows；非 Windows 目标会在编译时明确失败。
- Windows shell 在新建窗口或后台标签场景可能丢弃原始选中请求。项目会通过 Explorer 的文件夹视图补选并置前；如果当前 shell 不暴露该视图，结果可能退化为只打开目录。
- `newtab` 和 `overwrite` 是明确的行为取舍：前者保留用户当前标签但可能增加重复标签，后者不增加标签但会替换当前标签内容。
- 真实 Explorer 测试和时延测试依赖当前桌面状态，默认不会运行。

## 版本与许可

当前版本为 `0.1.0`。0.x 期间，公开 API、`reveal` 命令、`official-bindings` feature 和 `REVEAL_*` 环境变量均视为公开接口；破坏性变化会提升次版本号。

项目采用 MIT 许可，详见 [LICENSE-MIT](LICENSE-MIT)。变更记录见 [CHANGELOG.md](CHANGELOG.md)。
