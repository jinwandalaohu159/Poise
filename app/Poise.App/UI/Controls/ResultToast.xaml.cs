using System.Numerics;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Theme;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 一次操作的结果：浮在页面底部，从下方淡入，8 秒后或点关闭时淡出。数字从 0 递增，可带操作链接。
/// 已经显示时再来新结果，原地更新内容，不先收起再展开。
/// </summary>
public sealed partial class ResultToast : UserControl
{
    private static readonly TimeSpan AutoClose = TimeSpan.FromSeconds(8);
    /// <summary>出现前、消失后所在的位置：向下偏移。</summary>
    private static readonly Vector3 Away = new(0, 16, 0);
    /// <summary>阴影需要的抬升高度。</summary>
    private static readonly Vector3 Elevation = new(0, 0, 32);

    private readonly DispatcherTimer _timer = new() { Interval = AutoClose };
    private NumberRoller? _roller;
    private bool _shown;
    /// <summary>每次显示或隐藏都换一个；淡出结束时如果已经换了，说明又显示了，不再折叠。</summary>
    private object _generation = new();

    public ResultToast()
    {
        InitializeComponent();
        Surface.Translation = Elevation;
        _timer.Tick += (_, _) => Hide();
        Bar.Closing += (_, e) =>
        {
            // 由这里负责淡出；InfoBar 自己不收起。
            e.Cancel = true;
            Hide();
        };
        // 页面留在导航缓存里时（主页），离开再回来要接着计时，否则提示一直挂着。
        Unloaded += (_, _) => _timer.Stop();
        Loaded += (_, _) =>
        {
            if (_shown)
            {
                _timer.Start();
            }
        };
    }

    public async void Show(Feedback f)
    {
        Bar.Severity = f.Kind switch
        {
            FeedbackKind.Success => InfoBarSeverity.Success,
            FeedbackKind.Warning => InfoBarSeverity.Warning,
            FeedbackKind.Error => InfoBarSeverity.Error,
            _ => InfoBarSeverity.Informational,
        };
        Bar.Message = string.Join(Environment.NewLine, f.Details);
        Bar.ActionButton = f is { ActionLabel: { } label, Action: { } act }
            ? new HyperlinkButton { Content = label, Command = new RelayCommand(act) }
            : null;
        if (f.Count is { } count)
        {
            _roller = new NumberRoller(v => Bar.Title = count.Text(v), minStep: 0);
            _roller.RollFrom(0, count.To);
        }
        else
        {
            _roller = null;
            Bar.Title = f.Message;
        }
        _timer.Stop();
        _timer.Start();

        if (_shown)
        {
            return;
        }
        _shown = true;
        var generation = _generation = new object();
        if (Visibility == Visibility.Visible && Motion.Enabled)
        {
            // 正在淡出：从当前位置直接回来，不先跳到起点。
            Motion.EnableTransitions(this);
            Opacity = 1;
            Translation = Vector3.Zero;
            return;
        }
        Visibility = Visibility.Visible;
        if (!Motion.Enabled)
        {
            Opacity = 1;
            Translation = Vector3.Zero;
            return;
        }
        OpacityTransition = null;
        TranslationTransition = null;
        Opacity = 0;
        Translation = Away;
        await Motion.NextFrameAsync();
        if (generation != _generation)
        {
            return;
        }
        Motion.EnableTransitions(this);
        Opacity = 1;
        Translation = Vector3.Zero;
    }

    public async void Hide()
    {
        _timer.Stop();
        if (!_shown)
        {
            return;
        }
        _shown = false;
        var generation = _generation = new object();
        if (Motion.Enabled)
        {
            Motion.EnableTransitions(this, Motion.Exit);
            Opacity = 0;
            Translation = Away;
            await Task.Delay(Motion.Exit + TimeSpan.FromMilliseconds(16));
        }
        if (generation == _generation)
        {
            Visibility = Visibility.Collapsed;
        }
    }
}
