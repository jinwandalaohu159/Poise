using Poise.Core.Ipc;
using Poise.Core.Monitoring;

namespace Poise.Core.Tests;

/// <summary>修改发出时先显示的预期状态。</summary>
public sealed class ExpectedStatusTests
{
    private static Status Full => TestSupport.Fixture<Status>("status_full.json");

    [Fact]
    public void Choosing_a_concrete_mode_shows_it_as_the_mode_in_use()
    {
        var s = ExpectedStatus.WithMode(Full with { Mode = Mode.Quiet, EffectiveMode = Mode.Quiet }, Mode.Full);

        Assert.Equal((Mode.Full, Mode.Full), (s.Mode, s.EffectiveMode));
    }

    [Fact]
    public void Choosing_auto_keeps_the_current_mode_until_the_service_decides()
    {
        var s = ExpectedStatus.WithMode(Full with { Mode = Mode.Quiet, EffectiveMode = Mode.Quiet }, Mode.Auto);

        Assert.Equal((Mode.Auto, Mode.Quiet), (s.Mode, s.EffectiveMode));
    }

    [Fact]
    public void Fan_boost_is_clamped_and_other_settings_are_copied()
    {
        Assert.Equal(100, ExpectedStatus.WithFanBoost(Full, 150).FanBoost);
        Assert.Equal(GameMode.Off, ExpectedStatus.WithGameMode(Full, GameMode.Off).GameMode);
        Assert.True(ExpectedStatus.WithAutoClean(Full with { AutoClean = false }, true).AutoClean);
    }
}
