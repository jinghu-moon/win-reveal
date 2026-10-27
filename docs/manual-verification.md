# 人工验证指南

本指南用于在 Windows 桌面上验证真实 Explorer 行为，包括目标高亮、标签策略和路径处理。该验证会打开 Explorer 窗口或标签。只在可以操作当前桌面的会话中执行。

## 前置条件

- Windows 11 桌面会话，PowerShell 5.1 或更高版本。
- Rust 工具链满足项目 MSRV：Rust 1.88 或更高。
- 仓库已检出，且可运行 Cargo。

## 构建

默认手写 FFI 形态：

```powershell
cargo build --release
$exe = (Resolve-Path .\target\release\reveal.exe).Path
```

官方绑定形态可单独验证：

```powershell
cargo build --release --features official-bindings
$exe = (Resolve-Path .\target\release\reveal.exe).Path
```

两次构建会覆盖同一路径的 exe。若要分别保留产物，请在每次构建后复制并改名。

## 基本 reveal

在仓库根目录创建包含空格、中文和逗号的夹具：

```powershell
$dir = Join-Path (Get-Location) 'tmp\reveal-check\子 目录, x'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$file = Join-Path $dir '目标 文件.txt'
Set-Content -LiteralPath $file -Value 'test'
& $exe $file
$LASTEXITCODE
```

预期退出码为 `0`，Explorer 打开 `子 目录, x`，并高亮 `目标 文件.txt`。目标应处于选中状态，不应进入重命名编辑状态；窗口应恢复并置前。成功时 stderr 应为空。

## 标签行为

使用同一夹具依次检查以下状态：

1. 目标目录没有打开：执行 reveal 后应打开该目录并选中目标。
2. 目标目录处于当前可见标签：应复用该视图并选中目标。
3. 目标目录处于后台标签，未设置策略：默认 `newtab` 应保留原可见标签并将目标带到前台；可能为目标目录新增重复标签。
4. 再次建立后台目标目录，并设置覆盖策略：

   ```powershell
   $env:REVEAL_TAB_POLICY = 'overwrite'
   & $exe $file
   $LASTEXITCODE
   Remove-Item Env:\REVEAL_TAB_POLICY
   ```

   当前可见标签应导航到目标目录，标签总数不增加。此行为会替换当前标签原本显示的目录。

## CLI 错误与帮助

```powershell
& $exe --help
$LASTEXITCODE
& $exe
$LASTEXITCODE
& $exe 'C:\path\that\does-not-exist'
$LASTEXITCODE
```

帮助应退出 `0`，内容写入 stdout。缺参数、多个路径参数、空参数、不存在的路径和无父目录的盘根应退出 `2` 并将原因写入 stderr。shell HRESULT 错误使用退出码 `1`；稳定构造这类系统错误较困难，因此主要由错误映射的单元测试覆盖。

配置解析也可验证：将 `REVEAL_TAB_POLICY` 设为未支持值，或将 `REVEAL_DRY_RUN` 设为未支持值，应退出 `2` 并报告变量名。`--help` 会先于配置解析执行，因此错误配置不会阻止读取帮助。

## Dry-run 路径检查

dry-run 不调用 shell，可用于检查路径解析和归一化：

```powershell
$env:REVEAL_DRY_RUN = '1'
& $exe $file
$LASTEXITCODE
Remove-Item Env:\REVEAL_DRY_RUN
```

stdout 应包含 `would reveal: <绝对路径>`，退出码为 `0`。要检查 verbatim 路径，可以用反斜杠字符拼接，避免 shell 或终端对连续反斜杠的处理差异：

```powershell
$b = [char]92
$verbatim = $b + $b + '?' + $b + 'C:\Windows\notepad.exe'
$env:REVEAL_DRY_RUN = '1'
& $exe $verbatim
Remove-Item Env:\REVEAL_DRY_RUN
```

存在的 `\\?\C:\...` 路径应回显为普通盘路径。盘相对路径（例如 `C:Windows\notepad.exe`）按盘根锚定。不存在的路径仍会退出 `2`。

## 未出现高亮时的诊断

如果 Explorer 打开了目录但目标未高亮：

1. 记录 Windows 版本和 build，并检查是否只有目标目录打开、没有其他 UI 错误。
2. 通过 PowerShell Shell COM 对该视图调用 `FolderView.SelectItem` 作为阳性对照。
3. 若阳性对照可以选中，说明该 shell 的视图枚举或匹配路径与本工具补偿逻辑不兼容。
4. 若阳性对照也不能选中，则当前 Explorer/shell 环境没有按预期执行选择。

build 26100 上曾观察到单独调用 `SHOpenFolderAndSelectItems` 会打开目录但丢弃新视图的选择请求；该结果不代表其他 Windows 版本的行为。

## 清理

验证完成后检查 Explorer 中是否还有夹具目录标签，手动关闭这些标签，并删除仓库下 `tmp\reveal-check`。测试不会关闭与用户目录标签共用同一个窗口的 Explorer frame。
