using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 主页和托盘面板的操作：切换模式和开关、一键清理、一键优化、卡顿诊断、"建议重启"稍后提醒。
/// 显示内容由 <see cref="HomeViewBuilder"/> 换算，状态或语言变化时重新生成。
/// </summary>
public sealed class HomeViewModel : IDisposable
{
    private readonly AppHost _host;

    public HomeViewModel(AppHost host)
    {
        _host = host;
        Current = Rebuild();
        host.Status.Changed += OnChanged;
        Loc.Changed += OnChanged;
    }

    public HomeView Current { get; private set; }

    /// <summary>显示内容更新（在界面线程上触发）。</summary>
    public event Action<HomeView>? Updated;

    /// <summary>需要跳转到优化页（"去看看"）。</summary>
    public event Action? OpenOptimizations;

    private HomeView Rebuild() =>
        HomeViewBuilder.Build(_host.Status.Connected ? _host.Status.Latest : null, DateTimeOffset.Now);

    private void OnChanged()
    {
        Current = Rebuild();
        Updated?.Invoke(Current);
    }

    public Task SetModeAsync(Mode mode) => Run(() => _host.Api.SetModeAsync(mode));

    public Task SetAutoFullForGamesAsync(bool on) => Run(() => _host.Api.SetAutoFullForGamesAsync(on));

    public Task SetAutoCleanAsync(bool on) => Run(() => _host.Api.SetAutoCleanAsync(on));

    public Task SnoozeRestartAsync() => Run(_host.Api.SnoozeRestartAdviceAsync);

    private async Task Run(Func<Task> action)
    {
        try
        {
            await action();
            await _host.Status.RefreshAsync();
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            // 状态轮询会把"未连接"显示出来，这里不另外打扰用户。
            ErrorText.For(e);
        }
    }

    public async Task<Feedback> CleanAsync()
    {
        try
        {
            var r = await _host.Api.CleanNowAsync();
            await _host.Status.RefreshAsync();
            return CleanupText.Describe(r, _host.Status.Latest?.DryRun == true);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return Feedback.Error(ErrorText.For(e));
        }
    }

    public async Task<Feedback> OptimizeAsync()
    {
        try
        {
            var report = await _host.Api.ApplyRecommendedAsync();
            var items = await _host.Api.ListOptimizationsAsync();
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

    /// <summary>"刚才为什么卡"。服务没连上时返回 null。</summary>
    public async Task<DiagnoseView?> DiagnoseAsync()
    {
        try
        {
            return DiagnoseText.Build(await _host.Api.DiagnoseAsync());
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            ErrorText.For(e);
            return null;
        }
    }

    public void Dispose()
    {
        _host.Status.Changed -= OnChanged;
        Loc.Changed -= OnChanged;
    }
}
