using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 模式选择：自动、安静、均衡、全力，机型支持时加极限模式（G 模式等）。
/// 选中框标在用户选择的模式上，只在选择变化时滑动；首次显示和尺寸变化时直接就位。
/// 实际使用的模式与选择不同时（游戏时切换、过热降档），在那张卡片上标"使用中"；
/// 选择"自动"时，自动卡片的说明显示此刻使用的模式。
/// </summary>
public sealed partial class ModeSelector : UserControl
{
    private static readonly Dictionary<Mode, string> Glyphs = new()
    {
        [Mode.Auto] = "\uF4A5",
        [Mode.Quiet] = "\uE708",
        [Mode.Balanced] = "\uE9E9",
        [Mode.Full] = "\uE945",
        [Mode.Vendor] = "\uEC4A",
    };

    private readonly List<(ModeCard Card, Mode Mode)> _cards = [];
    private Mode _selected = Mode.Auto;
    private Mode _effective = Mode.Balanced;
    private Mode? _inUse;
    private string? _vendor;
    private bool _compact;
    private bool _built;

    public ModeSelector()
    {
        InitializeComponent();
        Pill.TranslationTransition = new Vector3Transition { Duration = Motion.Enabled ? Motion.Normal : TimeSpan.Zero };
    }

    /// <summary>用户点选了某个模式。</summary>
    public event Action<Mode>? ModeChosen;

    /// <summary>紧凑模式（托盘面板）：只显示图标和名称。</summary>
    public bool Compact
    {
        set
        {
            _compact = value;
            Root.ColumnSpacing = value ? 6 : 8;
            foreach (var (card, _) in _cards)
            {
                card.IsCompact = value;
            }
        }
    }

    /// <summary>
    /// 更新显示：用户选的模式、实际使用的模式（"自动"的说明用）、要标"使用中"的模式（没有时为 null）、
    /// 机型极限模式（不支持时为 null）。
    /// </summary>
    public void Show(Mode selected, Mode effective, Mode? inUse, string? vendor)
    {
        if (_built && selected == _selected && effective == _effective && inUse == _inUse && vendor == _vendor)
        {
            return;
        }
        _inUse = inUse;
        var rebuild = !_built || vendor != _vendor;
        var moved = selected != _selected;
        _selected = selected;
        _effective = effective;
        _vendor = vendor;
        if (rebuild)
        {
            Build();
        }
        Describe();
        UpdateVisuals(animate: moved && !rebuild);
    }

    /// <summary>按可用模式生成等宽的卡片。</summary>
    private void Build()
    {
        foreach (var (card, _) in _cards)
        {
            Root.Children.Remove(card);
        }
        _cards.Clear();
        Root.ColumnDefinitions.Clear();
        var options = ModeText.Options(_vendor, _selected, _effective);
        for (var i = 0; i < options.Count; i++)
        {
            var mode = options[i].Mode;
            Root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var card = new ModeCard { Glyph = Glyphs[mode], BadgeLabel = Loc.Get("mode.in_use"), IsCompact = _compact };
            card.Invoked += _ => Choose(mode);
            Grid.SetColumn(card, i);
            Root.Children.Add(card);
            _cards.Add((card, mode));
        }
        // 选中框放在最后，保证画在所有卡片上面。
        Root.Children.Remove(Pill);
        Root.Children.Add(Pill);
        _built = true;
    }

    /// <summary>名称与说明（"自动"的说明随实际模式变化）。</summary>
    private void Describe()
    {
        var options = ModeText.Options(_vendor, _selected, _effective);
        foreach (var (card, mode) in _cards)
        {
            var option = options.First(o => o.Mode == mode);
            card.Title = option.Title;
            card.Description = option.Description;
        }
    }

    private void Choose(Mode mode)
    {
        if (mode == _selected)
        {
            return;
        }
        _selected = mode;
        Describe();
        UpdateVisuals(animate: true);
        ModeChosen?.Invoke(mode);
    }

    /// <summary>
    /// 选中框靠布局定位：直接放进选中卡片所在的列，大小随列自动一致，不依赖窗口是否可见、是否已布局。
    /// 需要滑动时，先用位移把它留在原来那一列的位置，再过渡回 0，看起来是从旧卡片滑到新卡片。
    /// </summary>
    private void UpdateVisuals(bool animate)
    {
        var to = _cards.FindIndex(c => c.Mode == _selected);
        if (to < 0)
        {
            return;
        }
        var from = Grid.GetColumn(Pill);
        Grid.SetColumn(Pill, to);
        // 窗口不可见（托盘面板收起）时直接就位，不留到下次显示再滑。
        var slide = animate && Motion.Enabled && from != to && from < _cards.Count
                    && _cards[from].Card.ActualWidth > 0 && XamlRoot?.IsHostVisible == true;
        if (slide)
        {
            var dx = (float)_cards[from].Card.TransformToVisual(_cards[to].Card).TransformPoint(default).X;
            _ = SlideFromAsync(dx);
        }
        foreach (var (card, mode) in _cards)
        {
            card.IsSelected = mode == _selected;
            card.ShowBadge = mode == _inUse;
        }
    }

    /// <summary>从相对新位置 <paramref name="dx"/> 处滑回原位：先瞬间设好起点，下一帧再过渡到 0。</summary>
    private async Task SlideFromAsync(float dx)
    {
        var transition = Pill.TranslationTransition;
        Pill.TranslationTransition = null;
        Pill.Translation = new Vector3(dx, 0, 0);
        await Motion.NextFrameAsync();
        Pill.TranslationTransition = transition;
        Pill.Translation = Vector3.Zero;
    }
}
