# 生成安装程序 dist\Poise-Setup-<版本>.exe：一个文件，双击即可安装或升级。
# 用 Windows 自带的 IExpress 打包，不需要额外的打包工具。安装程序未签名，首次运行可能出现 SmartScreen 提示。
# 默认先运行 build.ps1；已经构建过时可以加 -SkipBuild。只生成文件，不安装、不改动系统。
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist\Poise'

if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build.ps1')
}
foreach ($file in 'poise-service.exe', 'poise-agent.exe', 'Poise.exe', 'version.txt') {
    if (-not (Test-Path (Join-Path $dist $file))) { throw "没有找到 $file，请先运行 scripts\build.ps1" }
}
$version = (Get-Content (Join-Path $dist 'version.txt') -Raw).Trim()
$output = Join-Path $root "dist\Poise-Setup-$version.exe"

# IExpress 不支持子目录，也不认非 ASCII 路径：程序文件整体压成 Poise.zip，在临时目录（ASCII 路径）里打包。
$staging = Join-Path $env:TEMP 'poise-package'
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force $staging | Out-Null
Write-Host '压缩程序文件…'
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($dist, (Join-Path $staging 'Poise.zip'), 'Optimal', $false)
foreach ($script in 'setup.ps1', 'install.ps1', 'uninstall.ps1') {
    Copy-Item (Join-Path $PSScriptRoot $script) $staging
}

$target = Join-Path $staging 'Poise-Setup.exe'
$sed = Join-Path $staging 'poise.sed'
$files = 'setup.ps1', 'install.ps1', 'uninstall.ps1', 'Poise.zip'
$strings = for ($i = 0; $i -lt $files.Count; $i++) { "FILE$i=`"$($files[$i])`"" }
$entries = for ($i = 0; $i -lt $files.Count; $i++) { "%FILE$i%=" }
@"
[Version]
Class=IEXPRESS
SEDVersion=3
[Options]
PackagePurpose=InstallApp
ShowInstallProgramWindow=0
HideExtractAnimation=1
UseLongFileName=1
InsideCompressed=0
CAB_FixedSize=0
CAB_ResvCodeSigning=0
RebootMode=N
InstallPrompt=%InstallPrompt%
DisplayLicense=%DisplayLicense%
FinishMessage=%FinishMessage%
TargetName=%TargetName%
FriendlyName=%FriendlyName%
AppLaunched=%AppLaunched%
PostInstallCmd=%PostInstallCmd%
AdminQuietInstCmd=%AdminQuietInstCmd%
UserQuietInstCmd=%UserQuietInstCmd%
SourceFiles=SourceFiles
[Strings]
InstallPrompt=
DisplayLicense=
FinishMessage=
TargetName=$target
FriendlyName=Poise Setup
AppLaunched=powershell.exe -NoProfile -ExecutionPolicy Bypass -File setup.ps1
PostInstallCmd=<None>
AdminQuietInstCmd=
UserQuietInstCmd=
$($strings -join "`r`n")
[SourceFiles]
SourceFiles0=$staging\
[SourceFiles0]
$($entries -join "`r`n")
"@ | Set-Content -Path $sed -Encoding ascii

Write-Host '生成安装程序…'
$iexpress = Start-Process (Join-Path $env:SystemRoot 'System32\iexpress.exe') -ArgumentList '/N', '/Q', $sed -Wait -PassThru
if ($iexpress.ExitCode -ne 0 -or -not (Test-Path $target)) { throw "IExpress 打包失败（退出码 $($iexpress.ExitCode)）" }
Move-Item $target $output -Force
Remove-Item $staging -Recurse -Force

$size = '{0:N0} MB' -f ((Get-Item $output).Length / 1MB)
Write-Host "完成：$output（$size）"
