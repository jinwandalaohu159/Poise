using System.Diagnostics;
using System.Numerics;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Hosting;

namespace Poise.App.UI.Theme;

/// <summary>
/// 让一个图标原地匀速旋转（例如"重新分析"的刷新图标），在合成线程上运行。
/// 停止时等转满当前这一圈再停，图标回到原位，不会停在半途或跳回去；所以至少会完整转一圈，
/// 即使操作瞬间完成，用户也能看到"确实刷新了"。系统关闭动画效果时不转。
/// </summary>
public sealed class Spin(FrameworkElement icon)
{
    private static readonly TimeSpan Turn = TimeSpan.FromMilliseconds(700);

    private readonly Stopwatch _clock = new();
    private Visual? _visual;

    public void Start()
    {
        if (!Motion.Enabled || _clock.IsRunning)
        {
            return;
        }
        _visual ??= ElementCompositionPreview.GetElementVisual(icon);
        _visual.CenterPoint = new Vector3((float)icon.ActualWidth / 2, (float)icon.ActualHeight / 2, 0);
        var compositor = _visual.Compositor;
        var turn = compositor.CreateScalarKeyFrameAnimation();
        turn.InsertKeyFrame(0, 0);
        turn.InsertKeyFrame(1, 360, compositor.CreateLinearEasingFunction());
        turn.Duration = Turn;
        turn.IterationBehavior = AnimationIterationBehavior.Forever;
        _visual.StartAnimation(nameof(Visual.RotationAngleInDegrees), turn);
        _clock.Restart();
    }

    /// <summary>转满当前这一圈后停下。</summary>
    public async Task StopAsync()
    {
        if (!_clock.IsRunning || _visual is null)
        {
            return;
        }
        var turns = Math.Max(1, Math.Ceiling(_clock.Elapsed / Turn));
        var remaining = Turn * turns - _clock.Elapsed;
        if (remaining > TimeSpan.Zero)
        {
            await Task.Delay(remaining);
        }
        _clock.Reset();
        _visual.StopAnimation(nameof(Visual.RotationAngleInDegrees));
        _visual.RotationAngleInDegrees = 0;
    }
}
