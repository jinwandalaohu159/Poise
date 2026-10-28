using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Settings;
using Poise.App.UI.Services;
using Poise.App.UI.Theme;

namespace Poise.App;

/// <summary>组合根：创建并持有全部长期存在的服务。界面通过它取用，不自行创建。</summary>
public sealed class AppHost : IDisposable
{
    private AppHost(DispatcherQueue dispatcher)
    {
        Dispatcher = dispatcher;
        Settings = UiSettings.Load();
        Loc.SetLanguage(Settings.Language);
        Theme = new ThemeManager(Settings);
        Client = new PoiseClient();
        Api = new PoiseApi(Client);
        Status = new StatusMonitor(Api);
        Presence = new UiPresence(Status, Api, dispatcher);
        Trend = new TrendFeed(Api, Presence, dispatcher);
    }

    public static AppHost Current { get; private set; } = null!;

    public DispatcherQueue Dispatcher { get; }

    public UiSettings Settings { get; }

    public ThemeManager Theme { get; }

    public PoiseClient Client { get; }

    public PoiseApi Api { get; }

    public StatusMonitor Status { get; }

    public UiPresence Presence { get; }

    /// <summary>走势图的数据（主页与卡顿诊断共用）。</summary>
    public TrendFeed Trend { get; }

    /// <summary>在界面线程上调用。</summary>
    public static AppHost Start(DispatcherQueue dispatcher) => Current = new AppHost(dispatcher);

    public void SetLanguage(string? language)
    {
        Settings.Language = language;
        Settings.Save();
        Loc.SetLanguage(language);
    }

    public void Dispose()
    {
        Trend.Dispose();
        Presence.Dispose();
        Status.Dispose();
        Client.Dispose();
    }
}
