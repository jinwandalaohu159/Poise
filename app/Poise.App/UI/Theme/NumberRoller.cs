using Microsoft.UI.Xaml.Media;

namespace Poise.App.UI.Theme;

/// <summary>
/// 数字滚动：从当前显示的值缓动到新值（减速曲线）。
/// 变化很小（不到一个 <c>minStep</c>，或不到当前值的 <see cref="MinFraction"/>：例行刷新里的小波动）、
/// 第一次显示或系统关闭了动画效果时直接跳到新值；动画中途收到新值会从当前位置继续。
/// 不是每帧都回调：最多每 <see cref="FrameInterval"/> 一次，并且看得出变化（跨过一个 <c>minStep</c>）
/// 时才回调——每次回调都要重新排版文字，高刷新率屏幕上每秒上百次没有必要。
/// </summary>
public sealed class NumberRoller(Action<double> render, TimeSpan? duration = null, double minStep = 1)
{
    /// <summary>两次回调至少隔这么久（约每秒 30 次）。</summary>
    private static readonly TimeSpan FrameInterval = TimeSpan.FromMilliseconds(33);

    /// <summary>变化不到当前值的这个比例时不滚动（每次滚动要在半秒多里每帧回调）。</summary>
    private const double MinFraction = 0.1;

    private readonly TimeSpan _duration = duration ?? TimeSpan.FromMilliseconds(600);
    private double? _shown;
    /// <summary>上次回调的值和时间。</summary>
    private double _rendered;
    private DateTime _renderedAt;
    private double _from;
    private double _to;
    private DateTime _start;
    private bool _rolling;

    /// <summary>当前显示的值（动画中为中间值）。</summary>
    public double? Shown => _shown;

    /// <summary>从 <paramref name="from"/> 开始滚动（例如清理结果从 0 递增）。</summary>
    public void RollFrom(double from, double target)
    {
        _shown = from;
        Render(from);
        RollTo(target);
    }

    public void RollTo(double target)
    {
        if (_shown is not { } current || !Motion.Enabled
            || Math.Abs(target - current) < Math.Max(minStep, Math.Abs(current) * MinFraction))
        {
            Stop();
            _shown = target;
            Render(target);
            return;
        }
        _from = current;
        _to = target;
        _start = DateTime.UtcNow;
        if (!_rolling)
        {
            _rolling = true;
            CompositionTarget.Rendering += OnRendering;
        }
    }

    /// <summary>清空（显示"—"之类的占位时）。</summary>
    public void Reset()
    {
        Stop();
        _shown = null;
    }

    private void OnRendering(object? sender, object e)
    {
        var now = DateTime.UtcNow;
        var t = Math.Min(1, (now - _start) / _duration);
        var eased = 1 - Math.Pow(1 - t, 3);
        _shown = _from + (_to - _from) * eased;
        if (t >= 1)
        {
            Stop();
            Render(_shown.Value);
            return;
        }
        var visible = minStep <= 0 || Math.Floor(_shown.Value / minStep) != Math.Floor(_rendered / minStep);
        if (visible && now - _renderedAt >= FrameInterval)
        {
            Render(_shown.Value);
        }
    }

    private void Render(double value)
    {
        _rendered = value;
        _renderedAt = DateTime.UtcNow;
        render(value);
    }

    private void Stop()
    {
        if (_rolling)
        {
            CompositionTarget.Rendering -= OnRendering;
            _rolling = false;
        }
    }
}
