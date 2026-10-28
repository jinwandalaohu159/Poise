using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;

namespace Poise.App.UI.Controls;

/// <summary>
/// 可排序的列名。<see cref="IsActive"/> 时文字用主色、显示箭头（向上 = 正序，向下 = 倒序），
/// 方向变化时箭头旋转半圈；不是当前列时箭头透明但仍占位（切换时列名不左右移动），文字为次要色。
/// 点了触发 <see cref="Click"/>，排序规则由页面决定。
/// </summary>
public sealed partial class SortHeader : UserControl
{
    private bool _active;
    private bool _descending;
    private bool _arrowLeading;

    public SortHeader()
    {
        InitializeComponent();
        foreach (var arrow in new[] { LeadingArrow, TrailingArrow })
        {
            arrow.RotationTransition = new ScalarTransition { Duration = Motion.Enabled ? Motion.Normal : TimeSpan.Zero };
            arrow.OpacityTransition = new ScalarTransition { Duration = Motion.Enabled ? Motion.Fast : TimeSpan.Zero };
        }
        ActualThemeChanged += (_, _) => Apply();
        Apply();
    }

    public event Action? Click;

    public string Text
    {
        get => Label.Text;
        set => Label.Text = value;
    }

    /// <summary>箭头放在文字左边（右对齐的数值列），默认在右边。</summary>
    public bool ArrowLeading
    {
        get => _arrowLeading;
        set
        {
            _arrowLeading = value;
            Apply();
        }
    }

    public bool IsActive
    {
        get => _active;
        set
        {
            _active = value;
            Apply();
        }
    }

    public bool Descending
    {
        get => _descending;
        set
        {
            _descending = value;
            Apply();
        }
    }

    /// <summary>悬停提示与读屏文字：点了之后会怎样排序。</summary>
    public string Hint
    {
        set
        {
            ToolTipService.SetToolTip(Root, value);
            AutomationProperties.SetHelpText(Root, value);
        }
    }

    private void Apply()
    {
        var shown = _arrowLeading ? LeadingArrow : TrailingArrow;
        var hidden = _arrowLeading ? TrailingArrow : LeadingArrow;
        hidden.Visibility = Visibility.Collapsed;
        shown.Visibility = Visibility.Visible;
        shown.Opacity = _active ? 1 : 0;
        shown.Rotation = _descending ? 180 : 0;
        var brush = (_active ? ActiveBrush : InactiveBrush).Background;
        Label.Foreground = brush;
        shown.Foreground = brush;
    }

    private void OnClick(object sender, RoutedEventArgs e) => Click?.Invoke();
}
