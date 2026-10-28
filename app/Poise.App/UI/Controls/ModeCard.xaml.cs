using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Poise.App.UI.Theme;
using Windows.System;

namespace Poise.App.UI.Controls;

/// <summary>
/// 一张模式卡。选中框由 <see cref="ModeSelector"/> 画在卡片上方并滑动；卡片自身的颜色不随选中变化，
/// 滑动途中不会出现文字与底色对比错乱。
/// </summary>
public sealed partial class ModeCard : UserControl
{
    private const double Hovered = 1;
    private const double Pressed = 0.5;

    private bool _selected;
    private bool _compact;

    public ModeCard()
    {
        InitializeComponent();
        Motion.EnableTransitions(HoverLayer, Motion.Instant);
        Motion.EnableTransitions(Badge, Motion.Fast);
        PointerEntered += (_, _) => HoverLayer.Opacity = Hovered;
        PointerExited += (_, _) => HoverLayer.Opacity = 0;
        AddHandler(PointerPressedEvent, new PointerEventHandler((_, _) => HoverLayer.Opacity = Pressed), true);
        AddHandler(PointerReleasedEvent, new PointerEventHandler((_, _) => HoverLayer.Opacity = Hovered), true);
        PointerCaptureLost += (_, _) => HoverLayer.Opacity = 0;
        Tapped += (_, _) => Invoke();
        KeyDown += OnKeyDown;
        // 卡片可能在加入界面之前就设好了尺寸；加入后再应用一次。
        Loaded += (_, _) => ApplySize();
    }

    public event Action<ModeCard>? Invoked;

    public string Glyph
    {
        get => Icon.Glyph;
        set => Icon.Glyph = value;
    }

    public string Title
    {
        get => TitleLabel.Text;
        set
        {
            TitleLabel.Text = value;
            AutomationProperties.SetName(this, value);
        }
    }

    public string Description
    {
        get => Desc.Text;
        set
        {
            Desc.Text = value;
            AutomationProperties.SetHelpText(this, value);
        }
    }

    public string BadgeLabel
    {
        get => BadgeText.Text;
        set => BadgeText.Text = value;
    }

    public bool IsSelected
    {
        get => _selected;
        set
        {
            if (_selected == value)
            {
                return;
            }
            _selected = value;
            FrameworkElementAutomationPeer.FromElement(this)?.RaisePropertyChangedEvent(
                SelectionItemPatternIdentifiers.IsSelectedProperty, !value, value);
        }
    }

    public bool IsCompact
    {
        get => _compact;
        set
        {
            _compact = value;
            ApplySize();
        }
    }

    private void ApplySize() => VisualStateManager.GoToState(this, _compact ? "Compact" : "Full", false);

    /// <summary>"自动"标记：以 0.8 → 1 的缩放弹入。</summary>
    public bool ShowBadge
    {
        set
        {
            Badge.CenterPoint = new Vector3((float)Badge.ActualWidth / 2, (float)Badge.ActualHeight / 2, 0);
            Badge.Opacity = value ? 1 : 0;
            Badge.Scale = value ? Vector3.One : new Vector3(0.8f, 0.8f, 1);
        }
    }

    private void Invoke() => Invoked?.Invoke(this);

    private void OnKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key is VirtualKey.Enter or VirtualKey.Space)
        {
            Invoke();
            e.Handled = true;
        }
    }

    protected override AutomationPeer OnCreateAutomationPeer() => new Peer(this);

    /// <summary>辅助技术（讲述人等）看到的是一组单选按钮：名称、说明、是否选中，可直接选择。</summary>
    private sealed partial class Peer(ModeCard owner) : FrameworkElementAutomationPeer(owner), ISelectionItemProvider
    {
        public bool IsSelected => owner.IsSelected;

        public IRawElementProviderSimple SelectionContainer => null!;

        public void Select() => owner.Invoke();

        public void AddToSelection() => owner.Invoke();

        public void RemoveFromSelection()
        {
            // 单选：只能通过选择另一项来取消。
        }

        protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.RadioButton;

        protected override string GetClassNameCore() => nameof(ModeCard);

        protected override object GetPatternCore(PatternInterface pattern) =>
            pattern == PatternInterface.SelectionItem ? this : base.GetPatternCore(pattern);
    }
}
