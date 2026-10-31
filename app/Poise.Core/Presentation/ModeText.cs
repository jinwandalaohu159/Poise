using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>模式卡片的一项：模式、名称、说明。</summary>
public sealed record ModeOption(Mode Mode, string Title, string Description);

/// <summary>游戏时使用哪个模式的一个选项。</summary>
public sealed record GameModeOption(GameMode Mode, string Title);

/// <summary>模式的名称与说明。机型极限模式按机型标识显示（"G 模式"）。</summary>
public static class ModeText
{
    /// <summary>
    /// 主页与托盘面板的模式卡片：自动、安静、均衡、全力，机型支持时加极限模式。
    /// 选中"自动"时，它的说明换成此刻实际使用的模式。
    /// </summary>
    public static IReadOnlyList<ModeOption> Options(string? vendor, Mode selected, Mode effective)
    {
        var modes = new List<Mode> { Mode.Auto, Mode.Quiet, Mode.Balanced, Mode.Full };
        if (vendor is not null)
        {
            modes.Add(Mode.Vendor);
        }
        return modes.Select(m => new ModeOption(m, Title(m, vendor),
            m == Mode.Auto && selected == Mode.Auto
                ? Loc.Format("mode.auto.current", Title(effective, vendor))
                : Description(m, vendor))).ToList();
    }

    /// <summary>
    /// 要标"使用中"的模式：只有玩游戏时切换、或温度过高自动降档，让实际模式与所选不同时才标。
    /// 选"自动"时说明里已写明当前模式，不标；其他差异（例如刚切换、硬件还没落地）也不标，避免标记一闪而过。
    /// </summary>
    public static Mode? InUse(Status s) =>
        s.Mode != Mode.Auto && s.EffectiveMode != s.Mode && (s.Health.Game is not null || s.Health.Code == HealthCode.Hot)
            ? s.EffectiveMode
            : null;

    public static string Title(Mode mode, string? vendor) => mode switch
    {
        Mode.Auto => Loc.Get("mode.auto"),
        Mode.Quiet => Loc.Get("mode.quiet"),
        Mode.Full => Loc.Get("mode.full"),
        Mode.Vendor => Loc.Get($"mode.vendor.{vendor}"),
        _ => Loc.Get("mode.balanced"),
    };

    private static string Description(Mode mode, string? vendor) => mode switch
    {
        Mode.Auto => Loc.Get("mode.auto.desc"),
        Mode.Quiet => Loc.Get("mode.quiet.desc"),
        Mode.Full => Loc.Get("mode.full.desc"),
        Mode.Vendor => Loc.Get($"mode.vendor.{vendor}.desc"),
        _ => Loc.Get("mode.balanced.desc"),
    };

    /// <summary>"游戏时使用"的选项：不切换、全力，机型支持时加极限模式。</summary>
    /// <summary>
    /// 下拉框里显示哪一项：机型不支持极限模式时，服务按全力处理，下拉框也显示全力。
    /// <paramref name="options"/> 来自 <see cref="GameOptions"/>。
    /// </summary>
    public static GameModeOption ShownGameOption(IReadOnlyList<GameModeOption> options, GameMode mode) =>
        options.FirstOrDefault(o => o.Mode == mode) ?? options.First(o => o.Mode == GameMode.Full);

    public static IReadOnlyList<GameModeOption> GameOptions(string? vendor)
    {
        var options = new List<GameModeOption>
        {
            new(GameMode.Off, Loc.Get("game_mode.off")),
            new(GameMode.Full, Title(Mode.Full, vendor)),
        };
        if (vendor is not null)
        {
            options.Add(new(GameMode.Vendor, Title(Mode.Vendor, vendor)));
        }
        return options;
    }
}
