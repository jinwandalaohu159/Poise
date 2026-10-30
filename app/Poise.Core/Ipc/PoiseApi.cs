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

    public Task SetGameModeAsync(GameMode mode) => Client.CallAsync("set_game_mode", new { mode });

    public Task SetAutoCleanAsync(bool enabled) => Client.CallAsync("set_auto_clean", new { enabled });
    public Task SetSmartScheduleAsync(bool enabled) => Client.CallAsync("set_smart_schedule", new { enabled });

    /// <summary>风扇加速，0～100（0 = 不加速）。</summary>
    public Task SetFanBoostAsync(int percent) => Client.CallAsync("set_fan_boost", new { percent = (byte)Math.Clamp(percent, 0, 100) });

    public Task<CleanupResult> CleanNowAsync() => Client.CallAsync<CleanupResult>("clean_now");

    /// <summary>清理页：按程序列出占用，以及每一项会怎样处理。</summary>
    public Task<CleanupOverview> CleanupOverviewAsync() => Client.CallAsync<CleanupOverview>("cleanup_overview");

    public Task<List<AppEntry>> ListAppsAsync() => Client.CallAsync<List<AppEntry>>("list_apps");

    public Task SetAppPreferenceAsync(string path, AppPreference preference) =>
        Client.CallAsync("set_app_preference", new { path, preference });

    public Task<List<ItemStatus>> ListOptimizationsAsync() => Client.CallAsync<List<ItemStatus>>("list_optimizations");

    public Task ApplyOptimizationAsync(string id) => Client.CallAsync("apply_optimization", new { id });

    public Task RevertOptimizationAsync(string id) => Client.CallAsync("revert_optimization", new { id });

    public Task<ApplyReport> ApplyRecommendedAsync() => Client.CallAsync<ApplyReport>("apply_recommended");

    public Task<List<string>> RevertAllAsync() => Client.CallAsync<List<string>>("revert_all");

    public Task<Diagnosis> DiagnoseAsync() => Client.CallAsync<Diagnosis>("diagnose");

    /// <summary>走势图的数据（只读、很轻）。</summary>
    public Task<Trend> GetTrendAsync() => Client.CallAsync<Trend>("trend");

    /// <summary>核心与缓存面板（只读、很轻）。</summary>
    public Task<CoreMap> GetCoresAsync() => Client.CallAsync<CoreMap>("cores");

    /// <summary>编号大于 <paramref name="after"/> 的日志（0 表示取最新的一批），从早到晚。</summary>
    public Task<EventPage> GetEventsAsync(ulong after) => Client.CallAsync<EventPage>("events", new { after });

    /// <summary>使用记录，最新的在前。</summary>
    public Task<List<SessionRecord>> GetRecordsAsync() => Client.CallAsync<List<SessionRecord>>("records");
}
