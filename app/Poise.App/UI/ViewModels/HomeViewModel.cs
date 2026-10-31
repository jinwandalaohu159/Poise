using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 主页和托盘面板的操作：切换模式和开关、一键清理、一键优化、"建议重启"稍后提醒。
/// 显示内容由 <see cref="HomeViewBuilder"/> 换算，状态或语言变化时重新生成。两边共用一个（见 <see cref="AppHost.Home"/>），
/// 各自在显示期间订阅 <see cref="Updated"/>。
/// </summary>
public sealed class HomeViewModel : IDisposable
{
    private readonly AppHost _host;

    private HomeView? _current;

    public HomeViewModel(AppHost host)
    {
        _host = host;
        host.Status.Changed += OnChanged;
        Loc.Changed += OnChanged;
    }

    /// <summary>当前的显示内容（状态或语言变了之后，第一次用到时才重新换算）。</summary>
    public HomeView Current => _current ??= Rebuild();

    /// <summary>已经拿到第一次状态。之前不要显示 <see cref="Current"/>（那只是"未连接"占位）。</summary>
    public bool Ready => _host.Status.HasResult;

    /// <summary>显示内容更新（在界面线程上触发）。</summary>
    public event Action<HomeView>? Updated;

    /// <summary>系统优化的进度（读取或一键优化后更新）。</summary>
    public event Action<OptimizeSummary>? OptimizeUpdated;

    /// <summary>需要跳转到系统优化页（提示条里的"查看"）。</summary>
    public event Action? OpenOptimizations;

    private HomeView Rebuild() =>
        HomeViewBuilder.Build(_host.Status.Connected ? _host.Status.Latest : null, DateTimeOffset.Now);

    private void OnChanged()
    {
        _current = null;
        // 没有人在看（托盘面板收起着）：先不换算。
        if (Ready && Updated is not null)
        {
            Updated(Current);
        }
    }

    public Task SetModeAsync(Mode mode) =>
        Run(() => _host.Api.SetModeAsync(mode), s => ExpectedStatus.WithMode(s, mode));

    public Task SetGameModeAsync(GameMode mode) =>
        Run(() => _host.Api.SetGameModeAsync(mode), s => ExpectedStatus.WithGameMode(s, mode));

    /// <summary>风扇加速，0～100（0 = 不加速）。</summary>
    public Task SetFanBoostAsync(int percent) =>
        Run(() => _host.Api.SetFanBoostAsync(percent), s => ExpectedStatus.WithFanBoost(s, percent));

    public Task SetAutoCleanAsync(bool on) =>
        Run(() => _host.Api.SetAutoCleanAsync(on), s => ExpectedStatus.WithAutoClean(s, on));

    public Task SnoozeRestartAsync() => Run(_host.Api.SnoozeRestartAdviceAsync);

    /// <summary>执行修改；有 <paramref name="expected"/> 时所有窗口先按预期状态显示，见 <see cref="StatusMonitor.ChangeAsync"/>。</summary>
    private async Task Run(Func<Task> action, Func<Status, Status>? expected = null)
    {
        try
        {
            await _host.Status.ChangeAsync(action, expected);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            // 状态轮询会把"未连接"显示出来，这里不另外打扰用户。
            ErrorText.Log(e);
        }
    }

    /// <summary>读取系统优化的进度。服务没连上时不更新。</summary>
    public async Task LoadOptimizeSummaryAsync()
    {
        try
        {
            OptimizeUpdated?.Invoke(OptimizeSummary.From(await _host.Optimizations.GetAsync()));
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            ErrorText.Log(e);
        }
    }

    public Task<Feedback> CleanAsync() => CleanupCommand.RunAsync(_host, _host.Status.RefreshAsync);

    public async Task<Feedback> OptimizeAsync()
    {
        try
        {
            var report = await _host.Api.ApplyRecommendedAsync();
            var items = await _host.Optimizations.ReloadAsync();
            OptimizeUpdated?.Invoke(OptimizeSummary.From(items));
            var slowPlugins = items.Count(i => i.Item.Group == "context_menu" && i.Item.SlowHint && !i.IsOptimized);
            var message = report.Applied.Count > 0 ? Loc.Format("optimize.done", report.Applied.Count) : Loc.Get("optimize.nothing");
            var details = slowPlugins > 0 ? new List<string> { Loc.Format("optimize.see_plugins", slowPlugins) } : [];
            return new Feedback(FeedbackKind.Success, message, details,
                slowPlugins > 0 ? Loc.Get("optimize.see") : null,
                slowPlugins > 0 ? () => OpenOptimizations?.Invoke() : null);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return Feedback.Error(ErrorText.For(e));
        }
    }

    public void Dispose()
    {
        _host.Status.Changed -= OnChanged;
        Loc.Changed -= OnChanged;
    }
}
