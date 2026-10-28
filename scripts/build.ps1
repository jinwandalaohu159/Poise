# 构建发布版本，输出到 dist\Poise：服务、托盘代理、界面放在同一目录（服务只接受同目录程序的连接）。
# 只编译、测试和复制文件，不安装、不改动系统。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist\Poise'
$staging = Join-Path $root 'dist\app'
$cargo = Join-Path $root 'service\Cargo.toml'
# 版本号只在一处维护（服务的 Cargo 工作区）：写进各程序的文件属性，安装时写进"设置 → 应用"。
$version = (Select-String -Path $cargo -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value

Write-Host '运行测试…'
cargo test -q --workspace --manifest-path $cargo
if ($LASTEXITCODE -ne 0) { throw '服务测试失败' }
dotnet test (Join-Path $root 'app\Poise.Core.Tests\Poise.Core.Tests.csproj') -nologo -v q
if ($LASTEXITCODE -ne 0) { throw '界面测试失败' }

Write-Host '构建服务与托盘代理…'
cargo build --release -p poise-service -p poise-agent --manifest-path $cargo
if ($LASTEXITCODE -ne 0) { throw '服务构建失败' }

Write-Host '构建界面…'
dotnet publish (Join-Path $root 'app\Poise.App\Poise.App.csproj') -c Release -r win-x64 --self-contained -o $staging -nologo "-p:Version=$version"
if ($LASTEXITCODE -ne 0) { throw '界面构建失败' }

if (Test-Path $dist) { Remove-Item $dist -Recurse -Force }
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item (Join-Path $staging '*') $dist -Recurse
foreach ($exe in 'poise-service.exe', 'poise-agent.exe') {
    Copy-Item (Join-Path $root "service\target\release\$exe") $dist
}
Remove-Item $staging -Recurse -Force

Set-Content -Path (Join-Path $dist 'version.txt') -Value $version -Encoding ascii

Write-Host "完成：$dist"
