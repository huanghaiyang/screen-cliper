<#
.SYNOPSIS
    构建 / 测试 ScreenCliper 的一键脚本。

.DESCRIPTION
    优先使用系统已安装的 Rust；若系统里没有，则自动回退到本仓库 `.toolchain`
    目录里的便携工具链（该目录未纳入版本控制）。

    当 CARGO_HOME 配置了本地 crates 源代理（`.toolchain/cargo/config.toml` 里的
    `sparse+http://127.0.0.1:47823/`）时，会自动确保代理进程在运行——这也是本机
    唯一的可用方式：该环境的 Windows Schannel 在受限进程里不可用，cargo 自带的
    libcurl 无法直接走 TLS，必须由 Node 代理承担 TLS。

.EXAMPLE
    .\build.ps1
    构建 release 版本：target\release\screen-cliper.exe

.EXAMPLE
    .\build.ps1 -Test
    运行单元测试

.EXAMPLE
    .\build.ps1 -Profile debug -SelfTest
    构建 debug 版本并跑一次离屏渲染自检（输出到 .toolchain\selftest）
#>
param(
    [ValidateSet("debug", "release")]
    [string] $Profile = "release",
    [switch] $Test,
    [switch] $SelfTest,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $Rest
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$localCargoHome = Join-Path $root ".toolchain\cargo"
$localCargo = Join-Path $localCargoHome "bin\cargo.exe"

# ---------------------------------------------------------------------------
# 1. 找到 cargo
# ---------------------------------------------------------------------------
if (Test-Path $localCargo) {
    $env:CARGO_HOME = $localCargoHome
    $env:RUSTUP_HOME = Join-Path $root ".toolchain\rustup"
    $env:PATH = "$(Join-Path $localCargoHome 'bin');$($env:PATH)"
    $cargo = $localCargo
    Write-Host "使用仓库内便携工具链：$localCargo" -ForegroundColor DarkGray
} elseif (Get-Command cargo -ErrorAction SilentlyContinue) {
    $cargo = "cargo"
    Write-Host "使用系统 cargo：$((Get-Command cargo).Source)" -ForegroundColor DarkGray
} else {
    throw "未找到 cargo。请安装 Rust 工具链（https://rustup.rs），或保留仓库内的 .toolchain 目录。"
}

# ---------------------------------------------------------------------------
# 2. 需要时启动本地 crates 源代理
# ---------------------------------------------------------------------------
$cargoConfig = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME "config.toml" } else { $null }
if ($cargoConfig -and (Test-Path $cargoConfig) -and
    (Select-String -Path $cargoConfig -Pattern "127\.0\.0\.1:47823" -Quiet)) {
    $healthy = $false
    try {
        $probe = & curl.exe -s -m 2 "http://127.0.0.1:47823/healthz" 2>$null
        $healthy = ($probe -eq "ok")
    } catch {
        $healthy = $false
    }
    if (-not $healthy) {
        Write-Host "启动本地 crates 源代理…" -ForegroundColor DarkGray
        Start-Process -FilePath "node" `
            -ArgumentList (Join-Path $root ".toolchain\cargo-proxy.js") `
            -WorkingDirectory $root -WindowStyle Hidden
        Start-Sleep -Milliseconds 900
    }
}

# ---------------------------------------------------------------------------
# 3. 构建 / 测试
# ---------------------------------------------------------------------------
if ($Test) {
    Write-Host "cargo test" -ForegroundColor Cyan
    & $cargo test @Rest
    exit $LASTEXITCODE
}

$buildArgs = @("build")
if ($Profile -eq "release") { $buildArgs += "--release" }
$buildArgs += $Rest
Write-Host "cargo $($buildArgs -join ' ')" -ForegroundColor Cyan
& $cargo @buildArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exe = Join-Path $root "target\$Profile\screen-cliper.exe"
if (Test-Path $exe) {
    $sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Host "构建完成：$exe（$sizeMb MiB）" -ForegroundColor Green
}

if ($SelfTest) {
    $out = Join-Path $root ".toolchain\selftest"
    Write-Host "运行离屏渲染自检 → $out" -ForegroundColor Cyan
    & $exe --selftest $out
}
