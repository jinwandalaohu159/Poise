<#
.SYNOPSIS
  录制一段游戏的帧时间并给出统计，用来比较"开着 Poise"和"退出 Poise"时帧数是否更稳。

.DESCRIPTION
  使用 PresentMon（Intel 开源的帧时间工具，首次运行自动从 GitHub 下载到 %LOCALAPPDATA%\Poise\tools）。
  需要以管理员身份运行（读取显示事件需要权限）。

  比较方法：同一个游戏、同一个场景，各录一次：
    .\frametime.ps1 -Process "League of Legends.exe" -Label off   # 先在托盘里退出 Poise
    .\frametime.ps1 -Process "League of Legends.exe" -Label on    # 再打开 Poise
  结果追加到 %LOCALAPPDATA%\Poise\frametime\summary.txt，两行对比即可。

  指标：
    平均帧率；1% / 0.1% 低帧（最慢的 1% / 0.1% 帧的平均帧率，越接近平均帧率越稳）；
    帧时间标准差（越小越稳）；卡顿次数（帧时间超过中位数 2 倍的帧）。
#>
param(
    [Parameter(Mandatory = $true)][string]$Process,
    [int]$Seconds = 60,
    [string]$Label = "run"
)

$ErrorActionPreference = "Stop"

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error "请以管理员身份运行（PresentMon 读取显示事件需要管理员权限）。"
}

$root = Join-Path $env:LOCALAPPDATA "Poise"
$tools = Join-Path $root "tools"
$out = Join-Path $root "frametime"
New-Item -ItemType Directory -Force $tools, $out | Out-Null

# ---- PresentMon ----
$presentMon = Get-ChildItem $tools -Filter "PresentMon*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $presentMon) {
    Write-Host "下载 PresentMon…"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $release = Invoke-RestMethod "https://api.github.com/repos/GameTechDev/PresentMon/releases/latest" -Headers @{ "User-Agent" = "Poise" }
    $asset = $release.assets | Where-Object { $_.name -match '^PresentMon-[\d.]+-x64\.exe$' } | Select-Object -First 1
    if (-not $asset) {
        Write-Error "没有找到 PresentMon 的 x64 命令行版本，请手动下载放到 $tools"
    }
    $target = Join-Path $tools $asset.name
    Invoke-WebRequest $asset.browser_download_url -OutFile $target -UseBasicParsing
    $presentMon = Get-Item $target
}

# ---- 录制 ----
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$csv = Join-Path $out "$Label-$stamp.csv"
Write-Host "录制 $Process，$Seconds 秒…（请保持在游戏里）"
& $presentMon.FullName --process_name $Process --timed $Seconds --terminate_after_timed `
    --output_file $csv --stop_existing_session --no_console_stats | Out-Null
if (-not (Test-Path $csv)) {
    Write-Error "没有录到数据：确认游戏正在运行，进程名是否为 $Process"
}

# ---- 统计 ----
$rows = Import-Csv $csv
if ($rows.Count -lt 10) {
    Write-Error "数据太少（$($rows.Count) 帧）"
}
$columns = $rows[0].PSObject.Properties.Name
$frameTimes = if ($columns -contains "MsBetweenPresents") {
    $rows | ForEach-Object { [double]$_.MsBetweenPresents }
} elseif ($columns -contains "FrameTime") {
    $rows | ForEach-Object { [double]$_.FrameTime }
} elseif (($columns -contains "CPUBusy") -and ($columns -contains "CPUWait")) {
    $rows | ForEach-Object { [double]$_.CPUBusy + [double]$_.CPUWait }
} else {
    Write-Error "无法识别 PresentMon 的输出列：$($columns -join ', ')"
}
$frameTimes = @($frameTimes | Where-Object { $_ -gt 0 })
$sorted = @($frameTimes | Sort-Object)
$count = $sorted.Count
$mean = ($frameTimes | Measure-Object -Average).Average
$median = $sorted[[int]($count / 2)]
$variance = ($frameTimes | ForEach-Object { ($_ - $mean) * ($_ - $mean) } | Measure-Object -Average).Average
$stddev = [Math]::Sqrt($variance)

function LowFps([double]$share) {
    $n = [Math]::Max(1, [int]($count * $share))
    $worst = $sorted[($count - $n)..($count - 1)]
    1000 / ($worst | Measure-Object -Average).Average
}

$stutters = @($frameTimes | Where-Object { $_ -gt 2 * $median }).Count
$summary = "{0}  {1,-8} 平均 {2:N1} fps  1% 低帧 {3:N1}  0.1% 低帧 {4:N1}  帧时间标准差 {5:N2} ms  卡顿 {6} 次（{7} 帧，{8} 秒）" -f `
    (Get-Date -Format "MM-dd HH:mm"), $Label, (1000 / $mean), (LowFps 0.01), (LowFps 0.001), $stddev, $stutters, $count, $Seconds

Write-Host $summary
Add-Content -Path (Join-Path $out "summary.txt") -Value $summary -Encoding UTF8
Write-Host "原始数据：$csv"
