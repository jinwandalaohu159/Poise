using Poise.Core.Ipc;

namespace Poise.Core.Monitoring;

/// <summary>
/// 用户修改设置后，状态"应当"变成的样子。修改请求发出的同时先按它更新界面，
/// 主窗口和托盘面板立即一致；服务确认后再以真实状态为准（失败时自然回到原值）。
/// </summary>
public static class ExpectedStatus
{
    /// <summary>选了具体模式：实际模式也先按它显示；选"自动"时实际模式要等服务换算，保持原值。</summary>
    public static Status WithMode(Status s, Mode mode) =>
        s with { Mode = mode, EffectiveMode = mode == Mode.Auto ? s.EffectiveMode : mode };

    public static Status WithGameMode(Status s, GameMode mode) => s with { GameMode = mode };

    public static Status WithFanBoost(Status s, int percent) => s with { FanBoost = Math.Clamp(percent, 0, 100) };

    public static Status WithAutoClean(Status s, bool on) => s with { AutoClean = on };

    public static Status WithSmartSchedule(Status s, bool on) => s with { SmartSchedule = on };
}
