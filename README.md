# Poise

让正在操作的程序始终反应迅速的 Windows 11 性能工具：后台让路的智能调度、一键清理、可还原的系统优化、按机型定制的硬件模式。

## 组成

| 部分 | 位置 | 说明 |
|---|---|---|
| 服务 | `service/crates/poise-service`（Rust） | 以 SYSTEM 身份运行，负责调度、清理、优化和硬件模式。所有改动都记入日志，可逐项还原；卸载时全部还原。 |
| 托盘代理 | `service/crates/poise-agent`（Rust，纯 Win32） | 用户会话中常驻（约 2 MB）：托盘图标与右键菜单；把用户桌面的状态（前台窗口、全屏、音频）上报给服务；按需启动界面。 |
| 界面 | `app/Poise.App`（C# / WinUI 3） | 主窗口与托盘面板。按需启动，都关闭后闲置 10 分钟自动退出。 |
| 界面逻辑 | `app/Poise.Core`（C#） | 与服务通信、文案表、状态换算、设置、日志；不依赖 WinUI，单元测试在 `app/Poise.Core.Tests`。 |
| 更新 | `service/crates/poise-update`（Rust） | 查询 GitHub 上的最新发布（托盘代理用）；更新助手 `poise-updater.exe`：下载、安装并重新启动新版本，显示进度。 |
| 安装程序 | `installer/`（Inno Setup） | 安装（可选位置）、升级、卸载。 |

服务与托盘代理、界面通过命名管道 `\\.\pipe\Poise` 通信。安装后，服务只接受与它位于同一目录（安装目录，只有管理员可写）的程序连接。托盘代理与界面之间用命名事件（`Local\Poise.App.*`）通信。

文案表（`app/Poise.Core/Localization/Strings`）由界面、托盘代理和更新助手共用；后两者在编译时嵌入。

## 开发

需要 Rust（stable）和 .NET 10 SDK。

```powershell
# 服务：观察模式（只判断、不执行，不改动系统），数据放在 %LOCALAPPDATA%\Poise\dev
cd service
cargo run -p poise-service -- console

# 服务：真实执行（需要管理员权限）
cargo run -p poise-service -- console --live

# 托盘代理（另开一个终端）。开发时用 POISE_UI_EXE 指向调试版界面
$env:POISE_UI_EXE = "$PWD\..\app\Poise.App\bin\Debug\net10.0-windows10.0.26100.0\win-x64\Poise.exe"
cargo run -p poise-agent

# 界面：单独运行时打开主窗口；--flyout 打开托盘面板
cd app\Poise.App
dotnet build
.\bin\Debug\net10.0-windows10.0.26100.0\win-x64\Poise.exe
```

测试与检查：

```powershell
cargo test --workspace              # service 目录
cargo clippy --workspace --all-targets
dotnet test app\Poise.Core.Tests    # 界面逻辑与契约测试
```

改了服务返回给界面的数据结构时，先运行 `$env:POISE_UPDATE_FIXTURES=1; cargo test -p poise-protocol --test contract` 重新生成契约样本，再运行界面测试确认界面能读取。

日志：服务 `ProgramData\Poise\poise.log`（开发时 `%LOCALAPPDATA%\Poise\dev`），托盘代理 `%LOCALAPPDATA%\Poise\agent.log`，界面 `%LOCALAPPDATA%\Poise\app.log`。服务的一轮采样超过 150 毫秒时，日志会写明最慢的是哪一步。

帧时间测量（判断 Poise 对游戏帧数的影响，管理员 PowerShell，首次运行自动下载 PresentMon）：同一游戏、同一场景各录一次，对比 `%LOCALAPPDATA%\Poise\frametime\summary.txt` 里的 1% 低帧、帧时间标准差和卡顿次数。

```powershell
.\scripts\frametime.ps1 -Process "game.exe" -Label off   # 先在托盘里退出 Poise
.\scripts\frametime.ps1 -Process "game.exe" -Label on    # 再打开 Poise
```

Poise 自身的原则：

- 不吃资源：服务和托盘代理以低于正常的优先级、效率模式运行在能效核上，一轮采样约 5 毫秒（每 2 秒一轮）；界面不显示（包括被全屏游戏挡住）时停止刷新并进入效率模式；托盘代理只在桌面状态变化时上报。
- 从不卡住调度：慢的操作都在各自的线程里——硬件控制（重新激活电源计划、BIOS 切换散热模式、风扇加速）、传感器读数、显卡占用测量、读取程序的友好名称（访问磁盘）。
- 少打扰硬件：机型接口（BIOS 的 WMI 方法）每次调用都可能让所有核心短暂停顿，界面不显示时 10 秒才读一次温度，玩游戏时只读处理器温度。
- 不和硬件抢方向盘：不按温度切换模式（处理器和显卡到了上限会自己降频保护）。

后台调度：按程序（整个进程树）判断，只在有人需要处理器时（正在用的程序在忙、整机较忙、用电池、玩游戏）才让持续忙碌的后台程序让路——降低优先级、开启效率模式、移到小核，内存与磁盘读写优先级也一起降低（Windows 不会因为程序在后台就降低这两项）；用户一点开就立即恢复。正在发声的程序不让路，其中真正输出声音的进程不受系统节能限速。每次让路、恢复、需求变化都记一条日志，写明程序和当时的负载。

游戏调度：游戏（自动识别的或标记的）及其进程树固定在大核上运行、不受系统节能限速，标记的游戏优先级高一级；玩游戏时后台程序更早让到小核（刚用过的客户端也不再豁免）；全力 / 极限模式只让大核全速，小核保持均衡的能效偏好，把功耗和温度留给游戏；一局结束后游戏模式保持 3 分钟，局间不来回切换散热模式。

## 构建与安装

```powershell
.\scripts\package.ps1      # 构建并生成安装程序 dist\Poise-Setup-<版本>.exe（加 -SkipBuild 只打包），不改动系统
.\scripts\build.ps1        # 运行测试，发布版输出到 dist\Poise，不改动系统
```

- 安装程序用 Inno Setup 6.7+ 编译（`installer/Poise.iss`；`winget install JRSoftware.InnoSetup`）。向导跟随系统的浅色 / 深色外观，按系统语言显示中文或英文；向导图片由 `python scripts/make-installer-images.py` 从 `icon/foam.png` 生成。
- 首次安装可以选择安装位置（默认 `Program Files\Poise`，不能放在用户文件夹里）；安装目录的权限会设为只有管理员可写——服务以 SYSTEM 身份运行，不能让别人替换其中的程序。再次运行安装程序即为升级：沿用上次的位置，只替换文件、更新并重启服务，保留设置、开机启动选择与优化记录。
- 服务的注册、首次安装时开启开机启动、卸载时还原系统修改，都由 `poise-service install / uninstall` 完成；安装程序只负责文件、快捷方式（开始菜单，可选桌面）和"设置 → 应用"里的登记。全程不出现控制台窗口。
- 开机启动是任务计划程序里 `\Poise\` 文件夹下的登录任务（定义见 `app/Poise.Core/Settings/AutostartTask.xml`，界面和安装共用），登录后立即启动托盘代理；不用 Run 键，因为 Windows 会把 Run 键里的程序推迟一两分钟。
- 卸载：在"设置 → 应用"里卸载 Poise。还原所有系统修改、删除服务、所有用户的开机启动任务、程序、记录与日志；如有修改没能还原，修改记录保留在 `ProgramData\Poise`，重启后重新安装再卸载即可重试。
- 发布：在 `service/Cargo.toml` 里改版本号，运行 `package.ps1`，再用 Project Hub 把安装程序发布到 GitHub Releases（标签 `v<版本>`，附件 `Poise-Setup-<版本>.exe`）。
- 自动更新：界面运行时启动后检查一次、之后每小时一次，托盘代理在后台定时检查；有新版本时托盘提醒，设置页"更新"里可以检查更新、立即更新。点"立即更新"后由更新助手 `poise-updater.exe`（复制到 `%LOCALAPPDATA%\Poise\updates` 运行）接手，界面退出：它在一个进度窗口里先下载安装程序（核对大小和 GitHub 提供的 SHA-256），再静默安装（会请求管理员权限）、等服务启动、重新启动托盘代理，完成后自动关闭；失败时说明原因，原来的版本保持可用。日志在 `%LOCALAPPDATA%\Poise\updater.log`。开发版本（没有 `version.txt`）不检查更新。
- 安装包未签名（没有代码签名证书），首次运行可能出现 SmartScreen 提示。
- 图标的原图是 `icon/foam.png`。更换后运行 `python scripts/make-icons.py`（需要 Pillow），重新生成 `app/Poise.App/Assets` 下的程序图标、托盘提醒图标和界面标志，再构建即可。

## 安全边界

- 对游戏只做任务管理器同等级别的操作（优先级、CPU 核心、效率模式）；不注入、不读写游戏内存、不 Hook、不修改游戏文件、不模拟输入；反作弊程序及其服务永不触碰。
- 不安装驱动；不关闭 Defender、防火墙、UAC、Windows 更新、内存完整性。
- 联网只为更新：界面和托盘代理访问 GitHub 查询最新版本、下载安装程序（校验 SHA-256），不上传任何数据；服务（SYSTEM）从不联网。
- 服务只执行固定的操作集合；管道有访问控制并拒绝远程连接；配置与日志只有管理员可写；重启用户程序时使用用户自己的身份。
- 戴尔 G15 的散热模式只使用 BIOS 自己列出的模式：安静 → 0xA3，全力（接通电源时）→ 0xA1，均衡 → 恢复启动时的原模式；服务停止或卸载时恢复原模式。
