using Microsoft.UI.Xaml.Media;

namespace Poise.App.UI.Theme;

/// <summary>
/// 数字滚动：从当前显示的值缓动到新值（减速曲线），每帧回调一次。
/// 变化很小、第一次显示或系统关闭了动画效果时直接跳到新值；动画中途收到新值会从当前位置继续。
/// </summary>
public sealed class NumberRoller(Action<double> render, TimeSpan? duration = null, double minStep = 1)
{
    private readonly TimeSpan _duration = duration ?? TimeSpan.FromMilliseconds(600);
    private double? _shown;
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
        render(from);
        RollTo(target);
    }

    public void RollTo(double target)
    {
        if (_shown is not { } current || !Motion.Enabled || Math.Abs(target - current) < minStep)
        {
            Stop();
            _shown = target;
            render(target);
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
        var t = Math.Min(1, (DateTime.UtcNow - _start) / _duration);
        var eased = 1 - Math.Pow(1 - t, 3);
        _shown = _from + (_to - _from) * eased;
        render(_shown.Value);
        if (t >= 1)
        {
            Stop();
        }
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
