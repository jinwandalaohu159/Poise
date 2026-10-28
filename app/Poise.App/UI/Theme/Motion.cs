using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI.ViewManagement;

namespace Poise.App.UI.Theme;

/// <summary>
/// 动效约定：只对位移、缩放、透明度做动画，交给合成线程执行；所有动画可被打断；
/// 不对按钮、卡片做按下缩放（文字和边框在亚像素间对齐会显得发抖），按下反馈只用颜色；
/// Windows 关闭"动画效果"时，位移和缩放改为瞬时，只保留极短的淡入淡出。
/// </summary>
public static class Motion
{
    public static readonly TimeSpan Instant = TimeSpan.FromMilliseconds(83);
    public static readonly TimeSpan Fast = TimeSpan.FromMilliseconds(167);
    public static readonly TimeSpan Normal = TimeSpan.FromMilliseconds(250);
    public static readonly TimeSpan Exit = TimeSpan.FromMilliseconds(150);
    /// <summary>数据驱动的变化（进度条长度、数字）：慢一些，读起来更从容。</summary>
    public static readonly TimeSpan Slow = TimeSpan.FromMilliseconds(400);

    private static readonly UISettings Settings = new();

    public static bool Enabled => Settings.AnimationsEnabled;

    /// <summary>给元素挂上隐式过渡：之后修改 Translation / Opacity / Scale 都会平滑过渡。</summary>
    public static void EnableTransitions(UIElement element, TimeSpan? duration = null)
    {
        var d = Enabled ? duration ?? Normal : TimeSpan.Zero;
        element.TranslationTransition = new Vector3Transition { Duration = d };
        element.ScaleTransition = new Vector3Transition { Duration = Enabled ? Fast : TimeSpan.Zero };
        element.OpacityTransition = new ScalarTransition { Duration = Enabled ? Fast : Instant };
    }

    /// <summary>只给缩放挂上隐式过渡（进度条用横向缩放表现长度）。</summary>
    public static void EnableScaleTransition(UIElement element, TimeSpan duration) =>
        element.ScaleTransition = new Vector3Transition { Duration = Enabled ? duration : TimeSpan.Zero };

    /// <summary>从 <paramref name="from"/> 倍缩放弹到原大小（完成对勾、标记出现）。</summary>
    public static async Task PopAsync(FrameworkElement element, float from = 0.6f)
    {
        element.ScaleTransition = null;
        element.CenterPoint = new Vector3((float)element.ActualWidth / 2, (float)element.ActualHeight / 2, 0);
        element.Scale = new Vector3(from, from, 1);
        if (!Enabled)
        {
            element.Scale = Vector3.One;
            return;
        }
        await NextFrameAsync();
        element.ScaleTransition = new Vector3Transition { Duration = Normal };
        element.Scale = Vector3.One;
    }

    /// <summary>
    /// 等到下一帧：先瞬间设好起点，下一帧再挂上过渡并设终点，动画才会从起点开始。
    /// </summary>
    public static Task NextFrameAsync()
    {
        var done = new TaskCompletionSource();
        var frames = 0;
        void OnRendering(object? sender, object e)
        {
            // 等两帧：确保起点已提交给合成线程。
            if (++frames < 2)
            {
                return;
            }
            CompositionTarget.Rendering -= OnRendering;
            done.TrySetResult();
        }
        CompositionTarget.Rendering += OnRendering;
        return done.Task;
    }
}
