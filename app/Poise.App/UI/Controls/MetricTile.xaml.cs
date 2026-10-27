using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Poise.Core.Ipc;
using Poise.App.UI.Theme;

namespace Poise.App.UI.Controls;

/// <summary>一个指标：数字变化时滚动到新值（600ms），颜色随等级变化。</summary>
public sealed partial class MetricTile : UserControl
{
    private static readonly TimeSpan RollDuration = TimeSpan.FromMilliseconds(600);

    private double? _shown;
    private double _from;
    private double _to;
    private DateTime _rollStart;
    private Func<double, string> _format = v => v.ToString("0");
    private bool _rolling;

    public MetricTile() => InitializeComponent();

    public string Label
    {
        get => LabelText.Text;
        set => LabelText.Text = value;
    }

    /// <summary>设置数值；null 显示"—"。变化不足 1 时不播放动画，避免数字一直抖。</summary>
    public void Set(double? value, Func<double, string> format, Level level, string tooltip)
    {
        _format = format;
        VisualStateManager.GoToState(this, level.ToString(), true);
        ToolTipService.SetToolTip(this, tooltip);
        AutomationProperties.SetName(this, $"{Label} {(value is { } v ? format(v) : "—")}");
        if (value is not { } target)
        {
            StopRolling();
            _shown = null;
            ValueText.Text = "—";
            return;
        }
        if (_shown is not { } current || !Motion.Enabled || Math.Abs(target - current) < 1)
        {
            StopRolling();
            _shown = target;
            ValueText.Text = format(target);
            return;
        }
        _from = current;
        _to = target;
        _rollStart = DateTime.UtcNow;
        if (!_rolling)
        {
            _rolling = true;
            CompositionTarget.Rendering += OnRendering;
        }
    }

    private void OnRendering(object? sender, object e)
    {
        var t = Math.Min(1, (DateTime.UtcNow - _rollStart) / RollDuration);
        var eased = 1 - Math.Pow(1 - t, 3);
        _shown = _from + (_to - _from) * eased;
        ValueText.Text = _format(_shown.Value);
        if (t >= 1)
        {
            StopRolling();
        }
    }

    private void StopRolling()
    {
        if (_rolling)
        {
            CompositionTarget.Rendering -= OnRendering;
            _rolling = false;
        }
    }
}
