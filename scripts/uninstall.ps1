# 卸载 Poise（需要管理员权限）。服务卸载时还原 Poise 做过的所有系统修改。
# 全部还原成功后才删除修改记录；有修改没能还原时保留程序和记录，可以稍后再运行本脚本重试。
#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$target = Join-Path $env:ProgramFiles 'Poise'
$service = Join-Path $target 'poise-service.exe'

if (-not (Test-Path $service)) { throw "没有找到 $service，Poise 可能没有安装" }

Get-Process -Name 'poise-agent', 'Poise' -ErrorAction SilentlyContinue | Stop-Process -Force

& $service uninstall
if ($LASTEXITCODE -ne 0) {
    throw '有修改没能还原，程序和修改记录都已保留。可以重启电脑后再运行一次本脚本。'
}

# 当前用户的开机启动、界面设置与日志。
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Poise' -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:LOCALAPPDATA 'Poise') -Recurse -Force -ErrorAction SilentlyContinue

Remove-Item (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Poise.lnk') -ErrorAction SilentlyContinue
# 修改已全部还原，记录不再需要。
Remove-Item (Join-Path $env:ProgramData 'Poise') -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $target -Recurse -Force

Write-Host 'Poise 已卸载，系统设置已全部还原，没有留下文件。'
