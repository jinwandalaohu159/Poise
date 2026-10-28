using Poise.Core.Diagnostics;
using Poise.Core.Ipc;

namespace Poise.Core.Monitoring;

/// <summary>
/// 定期向服务查询状态：窗口或托盘面板显示时每秒一次，否则每 10 秒一次。
/// 结果在创建它的线程（界面线程）上通过 <see cref="Changed"/> 通知。
/// <para>
/// 修改状态的操作（切换模式、开关）要经过 <see cref="ChangeAsync"/>，见 <see cref="RefreshGate"/>：
/// 否则正在路上的旧结果会让界面先跳回旧值、下一次查询再跳回来。
/// </para>
/// </summary>
public sealed class StatusMonitor : IDisposable
{
    private static readonly TimeSpan Visible = TimeSpan.FromSeconds(1);
    private static readonly TimeSpan Hidden = TimeSpan.FromSeconds(10);

    private readonly PoiseApi _api;
    private readonly SynchronizationContext _ui;
    private readonly Timer _timer;
    private readonly RefreshGate _gate = new();

    /// <summary>必须在界面线程上创建（需要它的 <see cref="SynchronizationContext"/>）。</summary>
    public StatusMonitor(PoiseApi api)
    {
        _api = api;
        _ui = SynchronizationContext.Current
              ?? throw new InvalidOperationException("StatusMonitor 必须在界面线程上创建");
        _timer = new Timer(_ => _ui.Post(_ => _ = RefreshAsync(), null), null, TimeSpan.Zero, Hidden);
    }

    public Status? Latest { get; private set; }

    public bool Connected { get; private set; }

    /// <summary>
    /// 是否已经有过一次查询结果（成功或确认连不上）。之前界面不应显示"未连接"之类的占位内容，
    /// 否则窗口一打开就会先显示占位、再切换到真实状态。
    /// </summary>
    public bool HasResult { get; private set; }

    /// <summary>在界面线程上触发。</summary>
    public event Action? Changed;

    public bool Fast
    {
        set
        {
            var interval = value ? Visible : Hidden;
            _timer.Change(interval, interval);
        }
    }

    /// <summary>
    /// 执行一次会改变状态的操作，完成后立即读取新状态。操作期间的查询结果不发布。
    /// 给出 <paramref name="expected"/> 时，先把预期的状态发布给所有订阅者（主窗口、托盘面板立即一致），
    /// 不等服务处理完；服务确认后以真实状态为准，失败时刷新会回到原值。
    /// </summary>
    public async Task ChangeAsync(Func<Task> change, Func<Status, Status>? expected = null)
    {
        if (expected is not null && Latest is { } current)
        {
            Latest = expected(current);
            Publish();
        }
        try
        {
            await _gate.ChangeAsync(change);
        }
        finally
        {
            await RefreshAsync();
        }
    }

    public Task RefreshAsync() => _gate.RefreshAsync(async () =>
    {
        try
        {
            var latest = await _api.GetStatusAsync();
            return () =>
            {
                Latest = latest;
                Connected = true;
                HasResult = true;
                Publish();
            };
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return () =>
            {
                Connected = false;
                HasResult = true;
                Publish();
            };
        }
    });

    private void Publish()
    {
        foreach (var handler in Changed?.GetInvocationList() ?? [])
        {
            try
            {
                ((Action)handler)();
            }
            catch (Exception e)
            {
                // 某个界面处理状态时出错：记下来，不影响其他订阅者和后续刷新。
                AppLog.Error("处理状态更新时出错", e);
            }
        }
    }

    public void Dispose() => _timer.Dispose();
}
