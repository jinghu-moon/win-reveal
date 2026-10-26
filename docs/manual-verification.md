# 手动验证步骤（真实弹窗与选中）

自动测试里唯一断言"用户看得见高亮"的手段是真弹窗，所以留一份人工复核清单。

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
9. 路径形态归一化（用 dry-run，**不开窗口**）。必须改写的约束列在
   [how-it-works.md](how-it-works.md) 的"路径形态归一化"一节。
   注意 `\\?\` 这种串经 git-bash/PowerShell 传参常被削掉一个反斜杠，所以用 `[char]92` 拼出来再传，
   别直接打字符串：

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
