<#
.SYNOPSIS
    下载与“在 Windows 资源管理器中选中文件”相关的 Rust crate 源码。
.DESCRIPTION
    优先使用 git clone；若无 git，则自动回退为下载 ZIP 并解压。
    所有源码保存在脚本所在目录的 rust_sources/ 下。
#>

$ErrorActionPreference = "Stop"

# 目标仓库列表（name -> GitHub URL）
$repos = [ordered]@{
    "showfile"                = "https://github.com/jf2048/showfile"
    "win-desktop-utils"       = "https://github.com/funwithcthulhu/win-desktop-utils"
    "ublx"                    = "https://github.com/Latka-Industries/UBLX"
    # martensite-share-platform 与 win_open 无独立 GitHub 仓库，见下方说明
}

$baseDir   = Join-Path $PSScriptRoot "rust_sources"
$useGit    = $null -ne (Get-Command git -ErrorAction SilentlyContinue)

function Write-Step($msg) { Write-Host ">>> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "  OK: $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "  WARN: $msg" -ForegroundColor Yellow }

if (-not (Test-Path $baseDir)) {
    New-Item -ItemType Directory -Path $baseDir | Out-Null
}

Write-Step "下载模式: $(if ($useGit) { 'Git clone' } else { 'ZIP 下载 + 解压' })"
Write-Step "目标目录: $baseDir"

foreach ($name in $repos.Keys) {
    $url     = $repos[$name]
    $destDir = Join-Path $baseDir $name

    Write-Step "处理 [$name] $url"

    if ($useGit) {
        if (Test-Path $destDir) {
            Write-Warn "目录已存在，跳过（如需重新下载请手动删除）: $destDir"
            continue
        }
        git clone --depth 1 $url $destDir
        if ($LASTEXITCODE -eq 0) { Write-Ok "$name 克隆完成" }
        else { Write-Warn "$name 克隆失败（exit code $LASTEXITCODE）" }
    }
    else {
        # 回退：下载 ZIP
        $zipPath = Join-Path $env:TEMP "$name.zip"
        $zipUrl  = "$url/archive/refs/heads/main.zip"

        try {
            Write-Host "  下载 ZIP: $zipUrl"
            Invoke-WebRequest -Uri $zipUrl -OutFile $zipPath -UseBasicParsing -ErrorAction Stop

            if (Test-Path $destDir) { Remove-Item $destDir -Recurse -Force }
            Expand-Archive -Path $zipPath -DestinationPath $baseDir -Force

            # GitHub ZIP 解压后目录名通常为 <repo>-main，重命名为统一名称
            $extracted = Get-ChildItem $baseDir -Directory | Where-Object {
                $_.Name -like "$name-*" -and $_.Name -ne $name
            } | Select-Object -First 1

            if ($extracted) {
                if (Test-Path $destDir) { Remove-Item $destDir -Recurse -Force }
                Rename-Item -Path $extracted.FullName -NewName $name
            }

            Remove-Item $zipPath -Force -ErrorAction SilentlyContinue
            Write-Ok "$name 下载并解压完成"
        }
        catch {
            Write-Warn "$name 下载失败: $($_.Exception.Message)"
        }
    }
}

Write-Step "全部处理完毕。源码位于: $baseDir"
Write-Host @"

说明:
  - showfile             → https://github.com/jf2048/showfile
  - win-desktop-utils    → https://github.com/funwithcthulhu/win-desktop-utils
  - ublx                 → https://github.com/Latka-Industries/UBLX
  - martensite-share-platform 与 win_open 无独立 GitHub 仓库；
    如需查看源码，可在 crates.io 下载 .crate 包，或在 docs.rs 的 "Source" 标签页查看。
"@ -ForegroundColor DarkGray