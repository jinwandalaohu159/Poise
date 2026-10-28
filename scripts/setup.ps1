# Poise 安装程序的入口：由安装包（package.ps1 生成的 Poise-Setup-*.exe）解压到临时目录后运行。
# 请求管理员权限，解压程序文件，再交给 install.ps1 安装或升级；最后用对话框告诉用户结果。
$ErrorActionPreference = 'Stop'
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    # 等管理员进程结束再退出：安装包在本进程退出后会删除解压出来的文件。
    try {
        $elevated = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
        exit $elevated.ExitCode
    } catch {
        # 用户在权限提示里选了"否"。
        exit 1223
    }
}

Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName System.IO.Compression.FileSystem
$payload = Join-Path $env:TEMP "Poise-Setup-$PID"
try {
    Write-Host '正在解压程序文件…'
    if (Test-Path $payload) { Remove-Item $payload -Recurse -Force }
    [IO.Compression.ZipFile]::ExtractToDirectory((Join-Path $PSScriptRoot 'Poise.zip'), $payload)
    Write-Host '正在安装…'
    & (Join-Path $PSScriptRoot 'install.ps1') -Source $payload
    [Windows.MessageBox]::Show('Poise 已安装，托盘图标已出现在任务栏。', 'Poise 安装程序', 'OK', 'Information') | Out-Null
    exit 0
} catch {
    [Windows.MessageBox]::Show("安装没有完成：$($_.Exception.Message)", 'Poise 安装程序', 'OK', 'Error') | Out-Null
    exit 1
} finally {
    Remove-Item $payload -Recurse -Force -ErrorAction SilentlyContinue
}
