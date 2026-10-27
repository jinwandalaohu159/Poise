namespace Poise.Core.Ipc;

/// <summary>
/// 服务提供的全部操作。界面只通过这里与服务交互，不拼方法名。
/// </summary>
public sealed class PoiseApi(PoiseClient client)
{
    public PoiseClient Client { get; } = client;

    public Task<Status> GetStatusAsync(CancellationToken ct = default) => Client.CallAsync<Status>("get_status", null, ct);

    /// <summary>界面窗口是否正显示着。显示期间需要定期重复上报（服务据此决定是否读取显卡）。</summary>
    public Task SetUiVisibleAsync(bool visible) => Client.CallAsync("set_ui_visible", new { visible });

    /// <summary>"建议重启"稍后再说。</summary>
    public Task SnoozeRestartAdviceAsync() => Client.CallAsync("snooze_restart_advice");

    public Task SetModeAsync(Mode mode) => Client.CallAsync("set_mode", new { mode });

    public Task SetAutoFullForGamesAsync(bool enabled) => Client.CallAsync("set_auto_full_for_games", new { enabled });

    public Task SetAutoCleanAsync(bool enabled) => Client.CallAsync("set_auto_clean", new { enabled });

    public Task<CleanupResult> CleanNowAsync() => Client.CallAsync<CleanupResult>("clean_now");

    public Task<List<AppEntry>> ListAppsAsync() => Client.CallAsync<List<AppEntry>>("list_apps");

    public Task SetAppPreferenceAsync(string path, AppPreference preference) =>
        Client.CallAsync("set_app_preference", new { path, preference });

    public Task<List<ItemStatus>> ListOptimizationsAsync() => Client.CallAsync<List<ItemStatus>>("list_optimizations");

    public Task ApplyOptimizationAsync(string id) => Client.CallAsync("apply_optimization", new { id });

    public Task RevertOptimizationAsync(string id) => Client.CallAsync("revert_optimization", new { id });

    public Task<ApplyReport> ApplyRecommendedAsync() => Client.CallAsync<ApplyReport>("apply_recommended");

    public Task<List<string>> RevertAllAsync() => Client.CallAsync<List<string>>("revert_all");

    public Task<StutterReport> DiagnoseAsync() => Client.CallAsync<StutterReport>("diagnose");
}
