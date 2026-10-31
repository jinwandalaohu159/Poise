using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Collections;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.Core.Settings;
using Poise.Core.Updates;

namespace Poise.App.UI.ViewModels;

/// <summary>设置页：语言、外观、开机启动、托盘图标提醒；这台电脑的硬件读数；更新；服务信息。</summary>
public sealed partial class SettingsViewModel : ObservableObject, IDisposable
{
    /// <summary>上次查到的开机启动状态：再打开设置页时先显示它，查到后再更新。</summary>
    private static bool _lastAutostart;

    private readonly AppHost _host;
    private readonly bool _ready;
    /// <summary>上次通知界面的服务信息（状态每秒刷新，没变时不通知）。</summary>
    private string? _serviceText;
    /// <summary>正在把查到的开机启动状态放到开关上（不是用户在切换）。</summary>
    private bool _syncingAutostart;

    public SettingsViewModel(AppHost host)
    {
        _host = host;
        LanguageIndex = host.Settings.Language switch { Loc.Chinese => 1, Loc.English => 2, _ => 0 };
        ThemeIndex = (int)host.Theme.Choice;
        AutostartEnabled = _lastAutostart;
        TrayAlertEnabled = host.Settings.TrayAlert;
        RefreshHardware();
        host.Status.Changed += RefreshHardware;
        host.Updates.Changed += RefreshUpdate;
        _ready = true;
        _ = LoadAutostartAsync();
    }

    public IReadOnlyList<string> LanguageOptions => [Loc.Get("settings.system"), "中文", "English"];

    public IReadOnlyList<string> ThemeOptions =>
        [Loc.Get("settings.system"), Loc.Get("settings.theme.light"), Loc.Get("settings.theme.dark")];

    [ObservableProperty]
    public partial int LanguageIndex { get; set; }

    [ObservableProperty]
    public partial int ThemeIndex { get; set; }

    [ObservableProperty]
    public partial bool AutostartEnabled { get; set; }

    /// <summary>托盘图标上的提醒圆点（托盘代理下次刷新图标时生效）。</summary>
    [ObservableProperty]
    public partial bool TrayAlertEnabled { get; set; }

    /// <summary>找不到托盘代理（开发时单独运行界面）时不能设置开机启动。</summary>
    public bool AutostartAvailable => Autostart.IsAvailable;

    /// <summary>这台电脑的硬件读数（每次状态刷新时原地更新数值）。</summary>
    public ObservableCollection<HardwareRowItem> Hardware { get; } = [];

    /// <summary>"Poise 0.2.1"，开发版本另有说明。</summary>
    public string VersionText => UpdateText.VersionLine(AppVersion.Installed);

    private UpdateState Update => _host.Updates.State;

    public string UpdateStatus => UpdateText.Status(Update);

    public string UpdateNotes => UpdateText.Notes(Update);

    public bool HasUpdateNotes => UpdateNotes.Length > 0;

    public Uri? ReleasePage => Update.Release?.PageUrl;

    public bool HasReleasePage => ReleasePage is not null && Update.Phase != UpdatePhase.Checking;

    public bool CanCheckUpdate => Update.CanCheck;

    public bool CanInstallUpdate => Update.CanInstall;

    public void CheckForUpdates() => _ = _host.Updates.CheckAsync();

    public void InstallUpdate() => _host.Updates.StartUpdate();

    private void RefreshUpdate()
    {
        foreach (var name in new[]
                 {
                     nameof(UpdateStatus), nameof(UpdateNotes), nameof(HasUpdateNotes), nameof(ReleasePage),
                     nameof(HasReleasePage), nameof(CanCheckUpdate), nameof(CanInstallUpdate),
                 })
        {
            OnPropertyChanged(name);
        }
    }

    public string ServiceText => _host.Client.IsConnected
        ? Loc.Format("settings.service.connected", _host.Client.ServiceVersion ?? "?")
        : Loc.Get("settings.service.disconnected");

    private void RefreshHardware()
    {
        var rows = _host.Status.Latest is { } s ? HardwareText.Rows(s) : [];
        // 已有的行只更新数值；多出、少了的行（显卡温度、风扇时有时无）单独增删，不重建整个列表。
        var existing = Hardware.ToDictionary(r => r.Label);
        var desired = rows.Select(r =>
        {
            if (existing.TryGetValue(r.Label, out var item))
            {
                item.Value = r.Value;
                return item;
            }
            return new HardwareRowItem(r);
        }).ToList();
        CollectionSync.Sync(Hardware, desired, r => r.Label);
        if (ServiceText != _serviceText)
        {
            _serviceText = ServiceText;
            OnPropertyChanged(nameof(ServiceText));
        }
    }

    partial void OnLanguageIndexChanged(int value)
    {
        if (_ready)
        {
            _host.SetLanguage(value switch { 1 => Loc.Chinese, 2 => Loc.English, _ => null });
        }
    }

    partial void OnThemeIndexChanged(int value)
    {
        if (_ready)
        {
            _host.Theme.Choice = (ThemeChoice)value;
        }
    }

    partial void OnAutostartEnabledChanged(bool value)
    {
        if (_ready && !_syncingAutostart)
        {
            _ = ApplyAutostartAsync(value);
        }
    }

    /// <summary>没设置成功时，开关回到实际状态。</summary>
    private async Task ApplyAutostartAsync(bool enabled)
    {
        if (await Autostart.SetAsync(enabled))
        {
            _lastAutostart = enabled;
        }
        else
        {
            await LoadAutostartAsync();
        }
    }

    /// <summary>查一次开机启动的实际状态，放到开关上（不当成用户的切换）。</summary>
    private async Task LoadAutostartAsync()
    {
        var enabled = await Task.Run(() => Autostart.IsEnabled);
        _lastAutostart = enabled;
        _syncingAutostart = true;
        AutostartEnabled = enabled;
        _syncingAutostart = false;
    }

    partial void OnTrayAlertEnabledChanged(bool value)
    {
        if (_ready)
        {
            _host.Settings.TrayAlert = value;
            _host.Settings.Save();
        }
    }

    public void Dispose()
    {
        _host.Status.Changed -= RefreshHardware;
        _host.Updates.Changed -= RefreshUpdate;
    }
}
