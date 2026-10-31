using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 一个指标：数值、颜色与进度条随等级变化。数值每秒更新，直接换成新值：每秒都滚一遍的话，
/// 界面开着时就一直在跑动画。和上次显示的一样时什么都不做；提示框只建一个，之后只换文字。
/// </summary>
public sealed partial class MetricTile : UserControl
{
    private readonly ToolTip _tip = new();
    private MetricView? _shown;

    public MetricTile()
    {
        InitializeComponent();
    }

    /// <summary>紧凑模式（托盘面板）：不显示进度条，数字与单位都小一号，单位仍与数字底边对齐。</summary>
    public bool IsCompact
    {
        set
        {
            Bar.Visibility = value ? Visibility.Collapsed : Visibility.Visible;
            if (value)
            {
                ValueText.FontSize = 22;
                ValueText.LineHeight = 28;
                UnitText.FontSize = 13;
                UnitText.Margin = new Thickness(0, 0, 0, 4);
            }
            else
            {
                ValueText.ClearValue(TextBlock.FontSizeProperty);
                ValueText.ClearValue(TextBlock.LineHeightProperty);
                UnitText.ClearValue(TextBlock.FontSizeProperty);
                UnitText.ClearValue(MarginProperty);
            }
        }
    }

    public void Show(MetricView m)
    {
        if (m == _shown)
        {
            return;
        }
        var before = _shown;
        _shown = m;
        LabelText.Text = m.Label;
        UnitText.Text = m.Unit;
        if (before?.Level != m.Level)
        {
            VisualStateManager.GoToState(this, m.Level.ToString(), true);
            Bar.SetLevel(m.Level == Poise.Core.Ipc.Level.Good ? null : m.Level);
        }
        if (before?.Tooltip != m.Tooltip)
        {
            _tip.Content = m.Tooltip;
            ToolTipService.SetToolTip(this, string.IsNullOrEmpty(m.Tooltip) ? null : _tip);
        }
        var name = $"{m.Label} {(m.Value is { } v ? $"{v:0}{m.Unit}" : "—")}";
        if (AutomationProperties.GetName(this) != name)
        {
            AutomationProperties.SetName(this, name);
        }
        Bar.Fraction = m.Fraction ?? 0;
        if (m.Value is { } value)
        {
            UnitText.Visibility = Visibility.Visible;
            ValueText.Text = value.ToString("0");
        }
        else
        {
            ValueText.Text = "—";
            UnitText.Visibility = Visibility.Collapsed;
        }
    }
}
