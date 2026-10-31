using Poise.Core.Monitoring;

namespace Poise.Core.Lifecycle;

/// <summary>
/// 服务报告"已暂停"（托盘代理没在上报：崩溃了、被结束了、没开开机启动）时把托盘代理拉起来。界面开着就能补上，
/// 不用等用户发现托盘图标没了。隔一阵才再试一次：它还在运行（只是连不上服务）时启动不起第二个。
/// </summary>
public sealed class AgentKeeper
{
    private static readonly TimeSpan RetryAfter = TimeSpan.FromSeconds(30);

    private readonly Action _start;
    private readonly Func<DateTimeOffset> _clock;
    private DateTimeOffset? _triedAt;

    /// <summary>跟着状态看：每次状态更新时检查一次。</summary>
    public AgentKeeper(StatusMonitor status) : this(() => Task.Run(Agent.EnsureRunning), () => DateTimeOffset.Now)
    {
        status.Changed += () => Observe(status.Latest?.Paused == true);
    }

    internal AgentKeeper(Action start, Func<DateTimeOffset> clock)
    {
        _start = start;
        _clock = clock;
    }

    /// <summary>看到一次状态：暂停着、离上次尝试够久时启动托盘代理。</summary>
    internal void Observe(bool paused)
    {
        var now = _clock();
        if (!paused || now - _triedAt < RetryAfter)
        {
            return;
        }
        _triedAt = now;
        _start();
    }
}
