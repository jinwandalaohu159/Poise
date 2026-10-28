using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>一个指标：数值变化时滚动到新值，颜色与进度条随等级变化。</summary>
public sealed partial class MetricTile : UserControl
{
    private readonly NumberRoller _roller;

    public MetricTile()
    {
        InitializeComponent();
        _roller = new NumberRoller(v => ValueText.Text = v.ToString("0"));
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
        LabelText.Text = m.Label;
        UnitText.Text = m.Unit;
        VisualStateManager.GoToState(this, m.Level.ToString(), true);
        ToolTipService.SetToolTip(this, string.IsNullOrEmpty(m.Tooltip) ? null : m.Tooltip);
        AutomationProperties.SetName(this, $"{m.Label} {(m.Value is { } v ? $"{v:0}{m.Unit}" : "—")}");
        Bar.SetLevel(m.Level == Poise.Core.Ipc.Level.Good ? null : m.Level);
        Bar.Fraction = m.Fraction ?? 0;
        if (m.Value is { } value)
        {
            UnitText.Visibility = Visibility.Visible;
            _roller.RollTo(value);
        }
        else
        {
            _roller.Reset();
            ValueText.Text = "—";
            UnitText.Visibility = Visibility.Collapsed;
        }
    }
}
