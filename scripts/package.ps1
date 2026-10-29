# 生成安装程序 dist\Poise-Setup-<版本>.exe：一个文件，双击即可安装或升级（可选安装位置），带卸载程序。
# 用 Inno Setup 6.7+ 编译 installer\Poise.iss（winget install JRSoftware.InnoSetup）。安装程序未签名，首次运行可能出现 SmartScreen 提示。
# 默认先运行 build.ps1；已经构建过时可以加 -SkipBuild。只生成文件，不安装、不改动系统。
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist\Poise'

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build.ps1')
}
foreach ($file in 'poise-service.exe', 'poise-agent.exe', 'poise-updater.exe', 'Poise.exe', 'AutostartTask.xml', 'version.txt') {
    if (-not (Test-Path (Join-Path $dist $file))) { throw "没有找到 $file，请先运行 scripts\build.ps1" }
}
$version = (Get-Content (Join-Path $dist 'version.txt') -Raw).Trim()

$iscc = @(
    (Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue).Source,
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw '没有找到 Inno Setup 6（ISCC.exe），请先运行：winget install JRSoftware.InnoSetup' }

Write-Host '生成安装程序…'
& $iscc /Qp "/DAppVersion=$version" "/DDist=$dist" "/DOutputDir=$(Join-Path $root 'dist')" (Join-Path $root 'installer\Poise.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup 编译失败（退出码 $LASTEXITCODE）" }

$output = Join-Path $root "dist\Poise-Setup-$version.exe"
$size = '{0:N0} MB' -f ((Get-Item $output).Length / 1MB)
Write-Host "完成：$output（$size）"
