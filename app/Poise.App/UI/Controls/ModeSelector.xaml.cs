using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.App.UI.Theme;

namespace Poise.App.UI.Controls;

/// <summary>
/// 三档模式选择：选中的强调色底块在卡片之间滑动（合成线程动画），
/// 游戏时自动切到全力会在"全力"卡片上弹出"自动"标记。
/// </summary>
public sealed partial class ModeSelector : UserControl
{
    private readonly (ModeCard Card, Mode Mode)[] _cards;
    private Mode _selected = Mode.Balanced;
    private Mode _effective = Mode.Balanced;

    public ModeSelector()
    {
        InitializeComponent();
        _cards = [(QuietCard, Mode.Quiet), (BalancedCard, Mode.Balanced), (FullCard, Mode.Full)];
        foreach (var (card, mode) in _cards)
        {
            card.Invoked += _ => Choose(mode);
        }
        Localize();
        Motion.EnableTransitions(Pill);
        Loaded += (_, _) => UpdateVisuals();
    }

    /// <summary>用户点选了某个模式。</summary>
    public event Action<Mode>? ModeChosen;

    /// <summary>紧凑模式（托盘面板）：隐藏描述文字。</summary>
    public bool Compact
    {
        set
        {
            foreach (var (card, _) in _cards)
            {
                card.IsCompact = value;
            }
        }
    }

    /// <summary>更新显示：用户选的模式与实际生效的模式（游戏时可能不同）。</summary>
    public void Show(Mode selected, Mode effective)
    {
        if (selected == _selected && effective == _effective)
        {
            return;
        }
        _selected = selected;
        _effective = effective;
        UpdateVisuals();
    }

    private void Localize()
    {
        QuietCard.Title = Loc.Get("mode.quiet");
        QuietCard.Description = Loc.Get("mode.quiet.desc");
        BalancedCard.Title = Loc.Get("mode.balanced");
        BalancedCard.Description = Loc.Get("mode.balanced.desc");
        FullCard.Title = Loc.Get("mode.full");
        FullCard.Description = Loc.Get("mode.full.desc");
        FullCard.BadgeLabel = Loc.Get("mode.auto_badge");
    }

    private void Choose(Mode mode)
    {
        _selected = mode;
        _effective = mode;
        UpdateVisuals();
        ModeChosen?.Invoke(mode);
    }

    private void OnSizeChanged(object sender, SizeChangedEventArgs e) => UpdateVisuals();

    private void UpdateVisuals()
    {
        var target = _cards.First(c => c.Mode == _effective).Card;
        if (target.ActualWidth <= 0)
        {
            return;
        }
        Pill.Width = target.ActualWidth;
        Pill.Height = target.ActualHeight;
        var x = target.TransformToVisual(Root).TransformPoint(default).X;
        Pill.Translation = new Vector3((float)x, 0, 0);
        foreach (var (card, mode) in _cards)
        {
            card.IsSelected = mode == _effective;
        }
        // 用户选的不是全力、但实际被游戏切到了全力。
        FullCard.ShowBadge = _effective == Mode.Full && _selected != Mode.Full;
    }
}
