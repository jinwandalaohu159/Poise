using Microsoft.UI.Dispatching;
using Poise.Core.Ipc;
using Poise.Core.Lifecycle;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Settings;
using Poise.Core.Updates;
using Poise.App.UI.Services;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;

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
        AgentKeeper = new AgentKeeper(Status);
        Presence = new UiPresence(Status, Api, dispatcher);
        Presence.AnyShown += Client.RetryNow;
        Trend = new TrendFeed(Api, Presence, dispatcher);
        Optimizations = new OptimizationStates(Api);
        Home = new HomeViewModel(this);
        Updates = new Updater(AppVersion.Installed, UpdateFiles.Default);
        UpdateSchedule = new UpdateSchedule(Updates, dispatcher);
    }

    public static AppHost Current { get; private set; } = null!;

    public DispatcherQueue Dispatcher { get; }

    public UiSettings Settings { get; }

    public ThemeManager Theme { get; }

    public PoiseClient Client { get; }

    public PoiseApi Api { get; }

    public StatusMonitor Status { get; }

    /// <summary>服务报告暂停时把托盘代理拉起来。</summary>
    private AgentKeeper AgentKeeper { get; }

    public UiPresence Presence { get; }

    /// <summary>走势图的数据（主页与卡顿诊断共用）。</summary>
    public TrendFeed Trend { get; }

    /// <summary>系统优化项的现状（主页与优化页共用）。</summary>
    public OptimizationStates Optimizations { get; }

    /// <summary>主页与托盘面板的显示内容和操作（两边共用一份，状态变了只换算一次）。</summary>
    public HomeViewModel Home { get; }

    /// <summary>检查新版本、启动更新助手（设置页显示状态）。</summary>
    public Updater Updates { get; }

    private UpdateSchedule UpdateSchedule { get; }

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
        Home.Dispose();
        UpdateSchedule.Dispose();
        Updates.Dispose();
        Trend.Dispose();
        Presence.Dispose();
        Status.Dispose();
        Client.Dispose();
    }
}
