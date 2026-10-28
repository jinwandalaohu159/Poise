using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Poise.App.UI.Controls;

/// <summary>
/// 说明提示：一个圆圈问号，指针移上去立即显示 <see cref="Text"/>（不等系统默认的悬停延迟），移开即收起；
/// 键盘聚焦时由系统的提示服务显示。没有说明时整个问号不显示。
/// </summary>
public sealed partial class InfoTip : UserControl
{
    public static readonly DependencyProperty TextProperty = DependencyProperty.Register(
        nameof(Text), typeof(string), typeof(InfoTip),
        new PropertyMetadata("", (d, e) => ((InfoTip)d).Apply((string)e.NewValue)));

    private readonly ToolTip _tip = new() { MaxWidth = 360 };

    public InfoTip()
    {
        InitializeComponent();
        // 默认没有说明：先收起，等设置了内容再显示（属性值没变时不会触发回调）。
        Visibility = Visibility.Collapsed;
        PointerEntered += (_, _) => _tip.IsOpen = HasText;
        PointerExited += (_, _) => _tip.IsOpen = false;
        PointerCanceled += (_, _) => _tip.IsOpen = false;
        Tapped += (_, _) => _tip.IsOpen = HasText && !_tip.IsOpen;
        // 离开页面时一定收起：提示仍打开而所在元素已被移出界面时，WinUI 可能崩溃。
        Unloaded += (_, _) => _tip.IsOpen = false;
    }

    public string Text
    {
        get => (string)GetValue(TextProperty);
        set => SetValue(TextProperty, value);
    }

    private bool HasText => !string.IsNullOrWhiteSpace(Text);

    private void Apply(string text)
    {
        var has = !string.IsNullOrWhiteSpace(text);
        _tip.Content = text;
        ToolTipService.SetToolTip(this, has ? _tip : null);
        AutomationProperties.SetHelpText(this, text);
        Visibility = has ? Visibility.Visible : Visibility.Collapsed;
    }
}
