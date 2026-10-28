# Poise

让正在操作的程序始终反应迅速的 Windows 11 性能工具：前台优先调度、一键清理、可还原的系统优化、按机型定制的硬件模式。需求见 [PRD.md](PRD.md)，界面规范见 [DESIGN.md](DESIGN.md)。

## 组成

| 部分 | 位置 | 说明 |
|---|---|---|
| 服务 | `service/crates/poise-service`（Rust） | 以 SYSTEM 身份运行，负责调度、清理、优化和硬件模式。所有改动都记入日志，可逐项还原；卸载时全部还原。 |
| 托盘代理 | `service/crates/poise-agent`（Rust，纯 Win32） | 用户会话中常驻（约 2 MB）：托盘图标与右键菜单；把用户桌面的状态（前台窗口、全屏、音频）上报给服务；按需启动界面。 |
| 界面 | `app/Poise.App`（C# / WinUI 3） | 主窗口与托盘面板。按需启动，都关闭后闲置 10 分钟自动退出。 |
| 界面逻辑 | `app/Poise.Core`（C#） | 与服务通信、文案表、状态换算、设置、日志；不依赖 WinUI，单元测试在 `app/Poise.Core.Tests`。 |

服务与托盘代理、界面通过命名管道 `\\.\pipe\Poise` 通信。安装后，服务只接受与它位于同一目录（`Program Files\Poise`，只有管理员可写）的程序连接。托盘代理与界面之间用命名事件（`Local\Poise.App.*`）通信。

文案表（`app/Poise.Core/Localization/Strings`）由界面和托盘代理共用；托盘代理在编译时嵌入。

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

日志：服务 `ProgramData\Poise\poise.log`（开发时 `%LOCALAPPDATA%\Poise\dev`），托盘代理 `%LOCALAPPDATA%\Poise\agent.log`，界面 `%LOCALAPPDATA%\Poise\app.log`。

## 构建与安装

```powershell
.\scripts\package.ps1      # 构建并生成安装程序 dist\Poise-Setup-<版本>.exe（加 -SkipBuild 只打包），不改动系统
.\scripts\build.ps1        # 运行测试，发布版输出到 dist\Poise，不改动系统
.\scripts\install.ps1      # 管理员 PowerShell：复制到 Program Files\Poise，安装并启动服务，启动托盘代理
.\scripts\uninstall.ps1    # 管理员 PowerShell：卸载服务并还原所有修改，然后删除程序、记录与日志
```

- 安装程序是单个 exe，用 Windows 自带的 IExpress 打包（不需要额外工具）：双击后请求管理员权限，自动安装或升级，结束时提示结果。
- 安装后桌面和开始菜单里有 Poise 快捷方式；在"设置 → 应用"里登记为 Poise，可以从那里卸载（效果同 `uninstall.ps1`）。
- 首次安装会为当前用户开启开机启动，之后由设置页控制。开机启动是任务计划程序里 `\Poise\` 文件夹下的登录任务（定义见 `app/Poise.Core/Settings/AutostartTask.xml`，界面和安装脚本共用），登录后立即启动托盘代理；不用 Run 键，因为 Windows 会把 Run 键里的程序推迟一两分钟。
- 再次运行 `install.ps1` 即为升级：只替换文件、重启服务，保留设置、开机启动选择与优化记录；旧版本写在 Run 键里的开机启动会迁移成登录任务。
- 发布：在 `service/Cargo.toml` 里改版本号，运行 `package.ps1`，再用 Project Hub 把安装程序发布到 GitHub Releases。
- 卸载时如有修改没能还原，程序和修改记录都会保留，可重启后再运行一次 `uninstall.ps1`。
- 安装包未签名（没有代码签名证书），首次运行可能出现 SmartScreen 提示。
- 图标的原图是 `icon/foam.png`。更换后运行 `python scripts/make-icons.py`（需要 Pillow），重新生成 `app/Poise.App/Assets` 下的程序图标、托盘提醒图标和界面标志，再构建即可。

## 安全边界

- 对游戏只做任务管理器同等级别的操作（优先级、CPU 核心、效率模式）；不注入、不读写游戏内存、不 Hook、不修改游戏文件、不模拟输入；反作弊程序及其服务永不触碰。
- 不安装驱动；不关闭 Defender、防火墙、UAC、Windows 更新、内存完整性；不联网。
- 服务只执行固定的操作集合；管道有访问控制并拒绝远程连接；配置与日志只有管理员可写；重启用户程序时使用用户自己的身份。
- 戴尔 G15 的散热模式只使用 BIOS 自己列出的模式：安静 → 0xA3，全力（接通电源时）→ 0xA1，均衡 → 恢复启动时的原模式；服务停止或卸载时恢复原模式。
