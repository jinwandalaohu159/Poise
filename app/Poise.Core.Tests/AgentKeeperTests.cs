using Poise.Core.Lifecycle;

namespace Poise.Core.Tests;

/// <summary>服务报告暂停时拉起托盘代理，隔一阵才再试。</summary>
public sealed class AgentKeeperTests
{
    [Fact]
    public void Starts_the_agent_only_while_paused_and_not_too_often()
    {
        var now = DateTimeOffset.Now;
        var starts = 0;
        var keeper = new AgentKeeper(() => starts++, () => now);

        keeper.Observe(false);
        Assert.Equal(0, starts);
        keeper.Observe(true);
        keeper.Observe(true);
        Assert.Equal(1, starts);
        now += TimeSpan.FromSeconds(31);
        keeper.Observe(true);
        Assert.Equal(2, starts);
    }
}
