; Poise 安装程序（Inno Setup 6.7+）。由 scripts\package.ps1 编译：
;   ISCC /DAppVersion=x.y.z /DDist=<dist\Poise> /DOutputDir=<dist> installer\Poise.iss
;
; - 首次安装可以选择安装位置（默认 Program Files\Poise）；升级时沿用上次的位置，不再询问。
; - 安装目录只有管理员可写（服务以 SYSTEM 身份运行，只接受同目录程序的连接，不能让别人替换其中的程序）。
; - 服务的注册、首次安装时开启开机启动、卸载时还原系统修改，都由 poise-service.exe 自己完成；
;   本脚本只负责文件、快捷方式和"设置 → 应用"里的登记。所有外部命令都隐藏运行，不出现控制台窗口。
; - 静默安装（/VERYSILENT，更新助手使用）不显示任何界面，也不启动托盘代理（由更新助手以用户身份启动）。

#ifndef AppVersion
  #error 请用 scripts\package.ps1 编译（需要 /DAppVersion、/DDist、/DOutputDir）
#endif

#define Images "images\"
#define Repo "https://github.com/jinwandalaohu159/Poise"

[Setup]
; AppId 决定"设置 → 应用"里的登记和升级识别，不能改。
AppId={{6B0E3D52-8C4A-4F7B-9E21-5A3C7D9F1B84}
AppName=Poise
AppVersion={#AppVersion}
AppVerName=Poise {#AppVersion}
AppPublisher=Poise
AppPublisherURL={#Repo}
AppSupportURL={#Repo}/issues
AppUpdatesURL={#Repo}/releases
VersionInfoVersion={#AppVersion}
VersionInfoProductName=Poise
VersionInfoDescription=Poise Setup
DefaultDirName={autopf}\Poise
DisableDirPage=auto
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
WizardStyle=modern dynamic windows11
WizardImageFile={#Images}wizard-100.png,{#Images}wizard-125.png,{#Images}wizard-150.png,{#Images}wizard-200.png,{#Images}wizard-250.png
WizardSmallImageFile={#Images}small-100.png,{#Images}small-125.png,{#Images}small-150.png,{#Images}small-200.png,{#Images}small-250.png
SetupIconFile=..\app\Poise.App\Assets\Poise.ico
UninstallDisplayIcon={app}\Poise.exe
UninstallDisplayName=Poise
OutputDir={#OutputDir}
OutputBaseFilename=Poise-Setup-{#AppVersion}
Compression=lzma2/max
SolidCompression=yes
; 关闭 Poise 由 PrepareToInstall 自己做（包括服务），不用"重新启动管理器"的提示框。
CloseApplications=no
RestartApplications=no
SetupLogging=yes
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage

[Languages]
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
chinesesimp.DesktopIcon=在桌面上创建快捷方式
english.DesktopIcon=Create a desktop shortcut
chinesesimp.LaunchPoise=启动 Poise
english.LaunchPoise=Start Poise
chinesesimp.ProtectFailed=无法设置安装目录的访问权限（只有管理员可写）：%1%n%n请选择其他安装位置。
english.ProtectFailed=Couldn't restrict the installation folder to administrators: %1%n%nPlease choose another location.
chinesesimp.UserFolder=请不要把 Poise 安装到用户文件夹里（例如桌面、下载、文档）。%n%nPoise 的服务以系统身份运行，安装目录需要只有管理员可以修改；用户文件夹做不到这一点。
english.UserFolder=Please don't install Poise inside a user folder (such as Desktop, Downloads or Documents).%n%nPoise's service runs as the system, so its folder must be changeable only by administrators, which a user folder can't guarantee.
chinesesimp.ServiceFailed=Poise 服务没有安装成功（代码 %1）。%n%n可以重新运行安装程序再试一次。
english.ServiceFailed=The Poise service couldn't be installed (code %1).%n%nYou can run Setup again to retry.
chinesesimp.RevertFailed=有系统修改没能还原，修改记录已保留在 %1。%n%n重启电脑后重新安装 Poise，再卸载一次即可重试还原。
chinesesimp.ServiceStuck=Poise 服务停不下来，安装没有进行，原来的版本保持不变。%n%n请稍后再试，或者重启电脑后再安装。
english.ServiceStuck=The Poise service didn't stop, so nothing was installed and the current version is unchanged.%n%nPlease try again later, or restart your PC and install again.
english.RevertFailed=Some system changes couldn't be reverted. The record of them is kept in %1.%n%nRestart your PC, reinstall Poise and uninstall it again to retry.

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"

[Files]
Source: "{#Dist}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
; 旧版本（脚本安装）留下的卸载脚本。
Type: files; Name: "{app}\uninstall.ps1"

[Icons]
Name: "{autoprograms}\Poise"; Filename: "{app}\Poise.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Poise"; Filename: "{app}\Poise.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
; 以普通权限（安装前的用户身份）启动托盘代理。
Filename: "{app}\poise-agent.exe"; Description: "{cm:LaunchPoise}"; Flags: nowait postinstall runasoriginaluser skipifsilent

[Code]
const
  ServiceName = 'Poise';
  { 旧版本（脚本安装）在"设置 → 应用"里的登记。 }
  LegacyUninstallKey = 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Poise';
  { 服务没有安装成功时安装程序的退出码（更新助手据此报告失败）。 }
  ServiceFailedExitCode = 10;
  { 等服务停下最多这么久（秒）：停止时要撤销调整、还原硬件模式，电源服务慢时要十几秒。 }
  StopWaitSeconds = 60;
  SC_MANAGER_CONNECT = $0001;
  SERVICE_QUERY_STATUS = $0004;
  SERVICE_STOPPED = 1;

type
  TServiceStatus = record
    dwServiceType: Longword;
    dwCurrentState: Longword;
    dwControlsAccepted: Longword;
    dwWin32ExitCode: Longword;
    dwServiceSpecificExitCode: Longword;
    dwCheckPoint: Longword;
    dwWaitHint: Longword;
  end;

var
  { 安装前停止了正在运行的服务：安装没有完成时重新启动它。 }
  ServiceWasRunning: Boolean;
  Installed: Boolean;
  ServiceExitCode: Integer;
  Reverted: Boolean;

function OpenSCManager(MachineName, DatabaseName: String; Access: Longword): THandle;
  external 'OpenSCManagerW@advapi32.dll stdcall';
function OpenService(Manager: THandle; ServiceName: String; Access: Longword): THandle;
  external 'OpenServiceW@advapi32.dll stdcall';
function QueryServiceStatus(Service: THandle; var Status: TServiceStatus): BOOL;
  external 'QueryServiceStatus@advapi32.dll stdcall';
function CloseServiceHandle(Handle: THandle): BOOL;
  external 'CloseServiceHandle@advapi32.dll stdcall';

{ 隐藏运行命令行程序并等它结束，返回退出码；没能启动时返回 -1。 }
function RunHidden(const FileName, Params: String): Integer;
var
  Code: Integer;
begin
  Log('运行 ' + FileName + ' ' + Params);
  if Exec(FileName, Params, '', SW_HIDE, ewWaitUntilTerminated, Code) then
    Result := Code
  else
    Result := -1;
  Log('退出码 ' + IntToStr(Result));
end;

function System32(const Exe: String): String;
begin
  Result := ExpandConstant('{sys}\' + Exe);
end;

{ 关闭所有用户会话里的托盘代理和界面。 }
procedure CloseApps;
begin
  RunHidden(System32('taskkill.exe'), '/F /IM poise-agent.exe /IM Poise.exe');
end;

{ 服务已经停下（或者没有这个服务、查不到）。 }
function ServiceStopped: Boolean;
var
  Manager, Service: THandle;
  Status: TServiceStatus;
begin
  Result := True;
  Manager := OpenSCManager('', 'ServicesActive', SC_MANAGER_CONNECT);
  if Manager = 0 then
    Exit;
  Service := OpenService(Manager, ServiceName, SERVICE_QUERY_STATUS);
  if Service <> 0 then begin
    if QueryServiceStatus(Service, Status) then
      Result := Status.dwCurrentState = SERVICE_STOPPED;
    CloseServiceHandle(Service);
  end;
  CloseServiceHandle(Manager);
end;

{ 等服务停下，最多 StopWaitSeconds 秒；返回停下了没有。 }
function WaitStopped: Boolean;
var
  I: Integer;
begin
  Result := ServiceStopped;
  I := 0;
  while not Result and (I < StopWaitSeconds * 2) do begin
    Sleep(500);
    Result := ServiceStopped;
    I := I + 1;
  end;
  Log('服务已停下：' + IntToStr(Ord(Result)));
end;

{ 停止服务，返回停之前它是否在运行（安装没有完成时据此重新启动它）。net stop 的退出码不作数：
  服务停得慢时它等不及就报失败，其实服务还在停止中——按服务的状态等它真正停下。
  先关掉"正常退出也按失败处理"：0.8.0 正常停止时报告的是失败的退出码，不关的话服务管理器 5 秒后
  又把它拉起来，文件替换不了。新版本 install 时会重新设好。 }
function StopService: Boolean;
begin
  Result := not ServiceStopped;
  if not Result then
    Exit;
  RunHidden(System32('sc.exe'), 'failureflag ' + ServiceName + ' 0');
  RunHidden(System32('net.exe'), 'stop ' + ServiceName);
end;

{ 安装目录只有管理员和系统可写，其他用户只读：所有者（含子项）改为 Administrators，
  整个目录先恢复为只有继承的权限，再给目录本身设置不继承的权限——子项随之继承这份权限。 }
function ProtectDir(const Dir: String): String;
var
  Icacls: String;
begin
  Result := '';
  if not ForceDirectories(Dir) then begin
    Result := Dir;
    Exit;
  end;
  Icacls := System32('icacls.exe');
  if (RunHidden(Icacls, '"' + Dir + '" /setowner *S-1-5-32-544 /T /C /Q') <> 0)
    or (RunHidden(Icacls, '"' + Dir + '" /reset /T /C /Q') <> 0)
    or (RunHidden(Icacls, '"' + Dir + '" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /Q') <> 0) then
    Result := 'icacls';
end;

function IsInside(const Path, Folder: String): Boolean;
var
  P, F: String;
begin
  P := AddBackslash(Lowercase(Path));
  F := AddBackslash(Lowercase(Folder));
  Result := (F <> '\') and (Copy(P, 1, Length(F)) = F);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpSelectDir)
    and (IsInside(WizardDirValue, GetEnv('USERPROFILE')) or IsInside(WizardDirValue, ExpandConstant('{%PUBLIC}'))) then
  begin
    SuppressibleMsgBox(CustomMessage('UserFolder'), mbError, MB_OK, IDOK);
    Result := False;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Failure: String;
begin
  Result := '';
  CloseApps;
  ServiceWasRunning := StopService;
  if ServiceWasRunning and not WaitStopped then begin
    { 停不下来：不安装（文件被占着，装到一半会回滚），原来的版本保持不变。 }
    Result := CustomMessage('ServiceStuck');
    Exit;
  end;
  Failure := ProtectDir(ExpandConstant('{app}'));
  if Failure <> '' then
    Result := FmtMessage(CustomMessage('ProtectFailed'), [Failure]);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    { 首次安装：注册并启动服务、开启开机启动；升级：更新服务的程序路径并启动。 }
    ServiceExitCode := RunHidden(ExpandConstant('{app}\poise-service.exe'), 'install');
    Installed := ServiceExitCode = 0;
    if not Installed then
      SuppressibleMsgBox(FmtMessage(CustomMessage('ServiceFailed'), [IntToStr(ServiceExitCode)]), mbError, MB_OK, IDOK);
    RegDeleteKeyIncludingSubkeys(HKLM64, LegacyUninstallKey);
  end;
end;

function GetCustomSetupExitCode: Integer;
begin
  if (ServiceExitCode <> 0) then
    Result := ServiceFailedExitCode
  else
    Result := 0;
end;

procedure DeinitializeSetup;
begin
  { 安装失败或被取消（文件已回滚）：让原来的版本继续工作。 }
  if ServiceWasRunning and not Installed then begin
    RunHidden(System32('sc.exe'), 'failureflag ' + ServiceName + ' 1');
    { 还在停止中时等它停下，否则启动不了。 }
    WaitStopped;
    RunHidden(System32('net.exe'), 'start ' + ServiceName);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  Data := ExpandConstant('{commonappdata}\Poise');
  case CurUninstallStep of
    usUninstall:
      begin
        CloseApps;
        { 停止并删除服务、还原所有系统修改、删除所有用户的开机启动任务。 }
        Reverted := RunHidden(ExpandConstant('{app}\poise-service.exe'), 'uninstall') = 0;
        if not Reverted then
          SuppressibleMsgBox(FmtMessage(CustomMessage('RevertFailed'), [Data]), mbError, MB_OK, IDOK);
      end;
    usPostUninstall:
      begin
        { 修改都已还原时，记录不再需要；没能还原时保留，之后可以重试。 }
        if Reverted then
          DelTree(Data, True, True, True);
        { 当前用户的界面设置、日志和下载的更新。 }
        DelTree(ExpandConstant('{localappdata}\Poise'), True, True, True);
      end;
  end;
end;
