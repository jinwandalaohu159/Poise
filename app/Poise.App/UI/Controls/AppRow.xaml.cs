using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;
using Windows.Foundation;

namespace Poise.App.UI.Controls;

/// <summary>
/// 程序偏好页的一行：程序（图标、名称与状态、CPU、内存、偏好）和展开后的各个进程。
/// <para>
/// 展开时进程区的高度从 0 平滑增长到内容高度，内容从上往下露出并淡入，下方的程序随之下移；收起时反过来。
/// 这里是动效约定（只动合成属性）的例外：只有改变布局高度，下方的行才能跟着平滑移动。
/// </para>
/// <para>
/// 列表是虚拟化的，行会被复用来显示别的程序：换程序时直接显示它的展开状态，不播动画；
/// 只有点展开箭头才播。
/// </para>
/// </summary>
public sealed partial class AppRow : UserControl
{
    public static readonly DependencyProperty ItemProperty = DependencyProperty.Register(
        nameof(Item), typeof(AppItem), typeof(AppRow),
        new PropertyMetadata(null, (d, e) => ((AppRow)d).OnItemChanged((AppItem?)e.OldValue, (AppItem?)e.NewValue)));

    private Storyboard? _transition;
    /// <summary>下一次展开状态变化是用户点的，播动画。</summary>
    private bool _animateNext;
    /// <summary>打开了偏好下拉框、暂停着刷新的程序。</summary>
    private AppItem? _holding;

    public AppRow()
    {
        InitializeComponent();
        Unloaded += (_, _) => ReleaseHold();
    }

    public AppItem? Item
    {
        get => (AppItem?)GetValue(ItemProperty);
        set => SetValue(ItemProperty, value);
    }

    private void OnItemChanged(AppItem? old, AppItem? item)
    {
        if (old is not null)
        {
            old.PropertyChanged -= OnItemPropertyChanged;
        }
        ReleaseHold();
        if (item is not null)
        {
            item.PropertyChanged += OnItemPropertyChanged;
        }
        _animateNext = false;
        ShowExpanded(animate: false);
    }

    private void OnItemPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(AppItem.IsExpanded) or nameof(AppItem.CanExpand))
        {
            ShowExpanded(_animateNext);
            _animateNext = false;
        }
    }

    private void OnToggle(object sender, RoutedEventArgs e)
    {
        _animateNext = true;
        Item?.Toggle();
    }

    private void ShowExpanded(bool animate)
    {
        var expanded = Item is { IsExpanded: true, CanExpand: true };
        Chevron.RotationTransition = animate && Motion.Enabled ? new ScalarTransition { Duration = Motion.Normal } : null;
        Chevron.Rotation = expanded ? 90 : 0;

        // 动画进行中又点了一次：从当前的高度和透明度接着往回走，不跳。
        var visible = ProcessesHost.Visibility == Visibility.Visible;
        var currentHeight = visible ? ProcessesHost.ActualHeight : 0;
        var currentOpacity = visible ? ProcessesContent.Opacity : 0;
        _transition?.Stop();
        _transition = null;
        if (!animate || !Motion.Enabled)
        {
            ShowFinal(expanded);
            return;
        }
        if (expanded)
        {
            // 先量出内容的完整高度，再长到这个高度；结束后恢复自动高度（之后进程增减时随内容变化）。
            ProcessesHost.Visibility = Visibility.Visible;
            ProcessesContent.Measure(new Size(ActualWidth, double.PositiveInfinity));
            Animate(currentHeight, ProcessesContent.DesiredSize.Height, currentOpacity, 1, Motion.Normal, EasingMode.EaseOut, expanded);
        }
        else
        {
            Animate(currentHeight, 0, currentOpacity, 0, Motion.Fast, EasingMode.EaseIn, expanded);
        }
    }

    /// <summary>动画结束（或不播动画）时的状态：展开时自动高度、完全不透明；收起时隐藏。</summary>
    private void ShowFinal(bool expanded)
    {
        ProcessesHost.Height = double.NaN;
        ProcessesContent.Opacity = 1;
        ProcessesHost.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
    }

    private void Animate(double fromHeight, double toHeight, double fromOpacity, double toOpacity, TimeSpan duration, EasingMode mode, bool expanded)
    {
        var ease = new CubicEase { EasingMode = mode };
        var height = new DoubleAnimation
        {
            From = fromHeight,
            To = toHeight,
            Duration = duration,
            EasingFunction = ease,
            EnableDependentAnimation = true,
        };
        Storyboard.SetTarget(height, ProcessesHost);
        Storyboard.SetTargetProperty(height, nameof(Height));
        var fade = new DoubleAnimation { From = fromOpacity, To = toOpacity, Duration = duration, EasingFunction = ease };
        Storyboard.SetTarget(fade, ProcessesContent);
        Storyboard.SetTargetProperty(fade, nameof(Opacity));

        var storyboard = new Storyboard { Children = { height, fade } };
        storyboard.Completed += (_, _) =>
        {
            if (ReferenceEquals(_transition, storyboard))
            {
                // 交还给布局：停止动画，设为终态。
                storyboard.Stop();
                _transition = null;
                ShowFinal(expanded);
            }
        };
        _transition = storyboard;
        ProcessesHost.Height = fromHeight;
        ProcessesContent.Opacity = fromOpacity;
        storyboard.Begin();
    }

    /// <summary>高度小于内容时只显示上面一截（从上往下露出）。</summary>
    private void OnHostSizeChanged(object sender, SizeChangedEventArgs e) =>
        ProcessesHost.Clip = new RectangleGeometry { Rect = new Rect(0, 0, e.NewSize.Width, e.NewSize.Height) };

    /// <summary>偏好下拉框打开期间暂停列表刷新（重新排序会移动甚至重建这一行）。</summary>
    private void OnDropDownOpened(object sender, object e)
    {
        ReleaseHold();
        _holding = Item;
        _holding?.HoldRefresh(true);
    }

    private void OnDropDownClosed(object sender, object e) => ReleaseHold();

    private void ReleaseHold()
    {
        _holding?.HoldRefresh(false);
        _holding = null;
    }
}
