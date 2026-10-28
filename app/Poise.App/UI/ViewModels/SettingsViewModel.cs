using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.Core.Settings;

namespace Poise.App.UI.ViewModels;

/// <summary>设置页：语言、外观、开机启动、托盘图标提醒；这台电脑的硬件读数；服务信息。</summary>
public sealed partial class SettingsViewModel : ObservableObject, IDisposable
{
    private readonly AppHost _host;
    private readonly bool _ready;

    public SettingsViewModel(AppHost host)
    {
        _host = host;
        LanguageIndex = host.Settings.Language switch { Loc.Chinese => 1, Loc.English => 2, _ => 0 };
        ThemeIndex = (int)host.Theme.Choice;
        AutostartEnabled = Autostart.IsEnabled;
        TrayAlertEnabled = host.Settings.TrayAlert;
        RefreshHardware();
        host.Status.Changed += RefreshHardware;
        _ready = true;
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

    /// <summary>这台电脑的硬件读数（每次状态刷新时更新）。</summary>
    public ObservableCollection<HardwareRow> Hardware { get; } = [];

    public string ServiceText => _host.Client.IsConnected
        ? Loc.Format("settings.service.connected", _host.Client.ServiceVersion ?? "?")
        : Loc.Get("settings.service.disconnected");

    private void RefreshHardware()
    {
        var rows = _host.Status.Latest is { } s ? HardwareText.Rows(s) : [];
        // 行数不变时原地更新，避免列表闪动。
        if (rows.Count == Hardware.Count)
        {
            for (var i = 0; i < rows.Count; i++)
            {
                if (Hardware[i] != rows[i])
                {
                    Hardware[i] = rows[i];
                }
            }
        }
        else
        {
            Hardware.Clear();
            foreach (var r in rows)
            {
                Hardware.Add(r);
            }
        }
        OnPropertyChanged(nameof(ServiceText));
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
        if (_ready)
        {
            _ = ApplyAutostartAsync(value);
        }
    }

    /// <summary>没设置成功时，开关回到实际状态。</summary>
    private async Task ApplyAutostartAsync(bool enabled)
    {
        if (!await Autostart.SetAsync(enabled))
        {
            AutostartEnabled = Autostart.IsEnabled;
        }
    }

    partial void OnTrayAlertEnabledChanged(bool value)
    {
        if (_ready)
        {
            _host.Settings.TrayAlert = value;
            _host.Settings.Save();
        }
    }

    public void Dispose() => _host.Status.Changed -= RefreshHardware;
}
