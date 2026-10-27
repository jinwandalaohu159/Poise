# 安装或升级 Poise（需要管理员权限）。先运行 build.ps1。
# - 文件复制到 Program Files\Poise（只有管理员可写，防止他人替换程序）
# - 安装并启动 Poise 服务（LocalSystem）
# - 首次安装时为当前用户开启开机启动（托盘代理），之后由设置页控制
# - 已安装时只替换文件并重启服务，保留设置、开机启动选择和优化记录
#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$source = Join-Path $root 'dist\Poise'
$target = Join-Path $env:ProgramFiles 'Poise'
$service = Join-Path $target 'poise-service.exe'
$agent = Join-Path $target 'poise-agent.exe'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

foreach ($exe in 'poise-service.exe', 'poise-agent.exe', 'Poise.exe') {
    if (-not (Test-Path (Join-Path $source $exe))) { throw "没有找到 $exe，请先运行 scripts\build.ps1" }
}

# 托盘代理和界面会占用文件，先关掉（所有用户的会话）。
Get-Process -Name 'poise-agent', 'Poise' -ErrorAction SilentlyContinue | Stop-Process -Force
$existing = Get-Service -Name 'Poise' -ErrorAction SilentlyContinue
if ($existing -and $existing.Status -ne 'Stopped') {
    Stop-Service -Name 'Poise'
    $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(20))
}

New-Item -ItemType Directory -Force $target | Out-Null
Copy-Item (Join-Path $source '*') $target -Recurse -Force

if ($existing) {
    Start-Service -Name 'Poise'
} else {
    & $service install
    if ($LASTEXITCODE -ne 0) { throw '服务安装失败' }
    # 管理员身份与登录用户相同（通常如此）时，写入的是登录用户的开机启动。
    New-ItemProperty -Path $runKey -Name 'Poise' -Value "`"$agent`"" -PropertyType String -Force | Out-Null
}

# 开始菜单快捷方式（所有用户）。
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Poise.lnk'
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = Join-Path $target 'Poise.exe'
$link.WorkingDirectory = $target
$link.Save()

# 以普通权限启动托盘代理：经由资源管理器启动，不继承本脚本的管理员权限。
Start-Process -FilePath 'explorer.exe' -ArgumentList "`"$agent`""

if ($existing) { Write-Host "Poise 已升级（$target）。" } else { Write-Host "Poise 已安装到 $target，托盘图标已出现在任务栏。" }
