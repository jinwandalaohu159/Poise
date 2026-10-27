using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Windows.UI.ViewManagement;

namespace Poise.App.UI.Theme;

/// <summary>
/// 动效约定：只对位移、缩放、透明度做动画，交给合成线程执行；所有动画可被打断；
/// Windows 关闭"动画效果"时，位移和缩放改为瞬时，只保留极短的淡入淡出。
/// </summary>
public static class Motion
{
    public static readonly TimeSpan Instant = TimeSpan.FromMilliseconds(83);
    public static readonly TimeSpan Fast = TimeSpan.FromMilliseconds(167);
    public static readonly TimeSpan Normal = TimeSpan.FromMilliseconds(250);
    public static readonly TimeSpan Exit = TimeSpan.FromMilliseconds(150);

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

    /// <summary>按下时缩到 0.98，松开回弹；鼠标移出再松开则视为取消。</summary>
    public static void AddPressFeedback(FrameworkElement element)
    {
        EnableTransitions(element);
        element.SizeChanged += (_, _) =>
            element.CenterPoint = new Vector3((float)element.ActualWidth / 2, (float)element.ActualHeight / 2, 0);
        void Down(object _, PointerRoutedEventArgs __) => element.Scale = new Vector3(0.98f, 0.98f, 1);
        void Up(object _, PointerRoutedEventArgs __) => element.Scale = Vector3.One;
        element.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler(Down), true);
        element.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler(Up), true);
        element.PointerExited += Up;
        element.PointerCanceled += Up;
        element.PointerCaptureLost += Up;
    }

    /// <summary>内容切换：旧内容上移淡出，执行 <paramref name="swap"/>，新内容从下方淡入。</summary>
    public static async Task SwapAsync(UIElement element, Action swap, float offset = 8)
    {
        if (!Enabled)
        {
            swap();
            return;
        }
        element.Opacity = 0;
        element.Translation = new Vector3(0, -offset, 0);
        await Task.Delay(Exit);
        swap();
        element.TranslationTransition = null;
        element.Translation = new Vector3(0, offset, 0);
        await Task.Delay(16);
        EnableTransitions(element);
        element.Opacity = 1;
        element.Translation = Vector3.Zero;
    }
}
