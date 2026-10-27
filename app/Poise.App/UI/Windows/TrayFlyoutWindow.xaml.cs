using System.Numerics;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Poise.Core.Localization;
using Poise.App.UI.Theme;
using Poise.Core.Presentation;
using Poise.App.UI.ViewModels;
using Windows.Graphics;

namespace Poise.App.UI.Windows;

/// <summary>
/// 托盘面板：状态、三档模式、一键清理。贴着任务栏弹出，点别处就收起。
/// 只创建一次，之后显示/隐藏复用。点托盘图标时由托盘代理唤起（见 App）。
/// </summary>
public sealed partial class TrayFlyoutWindow : Window
{
    private const int Width = 360;
    /// <summary>首次显示前还没有布局结果时使用的高度。</summary>
    private const int FallbackHeight = 280;
    private const int EdgeGap = 12;
    /// <summary>点托盘图标会先让面板失去焦点而收起，紧接着的点击不应再把它打开。</summary>
    private static readonly TimeSpan ReopenGuard = TimeSpan.FromMilliseconds(300);

    private readonly AppHost _host;
    private readonly HomeViewModel _vm;
    private DateTime _hiddenAt = DateTime.MinValue;
    private string? _shownTitle;

    public TrayFlyoutWindow(AppHost host)
    {
        _host = host;
        _vm = new HomeViewModel(host);
        InitializeComponent();
        Title = "Poise";
        SystemBackdrop = new DesktopAcrylicBackdrop();
        ConfigureChrome();

        Modes.Compact = true;
        Modes.ModeChosen += mode => _ = _vm.SetModeAsync(mode);
        CleanButton.IsAccent = true;
        CleanButton.Glyph = "\uE74D";
        CleanButton.Action = CleanAsync;
        Localize();
        Motion.EnableTransitions(Root);
        Motion.EnableTransitions(StatusText);
        ResetEntrance();
        // 窗口高度跟随内容：语言、描述换行变化时自动校正。
        Root.SizeChanged += (_, e) =>
        {
            if (Math.Abs(e.NewSize.Height - e.PreviousSize.Height) > 0.5)
            {
                PlaceAboveTaskbar();
            }
        };

        _vm.Updated += Render;
        Loc.Changed += Localize;
        host.Theme.Changed += ApplyTheme;
        Activated += OnActivated;
        Closed += OnClosed;
        Render(_vm.Current);
        ApplyTheme();
    }

    /// <summary>用户要求打开主窗口。</summary>
    public event Action? OpenMainRequested;

    public bool IsShown => AppWindow.IsVisible;

    public void Toggle()
    {
        if (IsShown)
        {
            Hide();
        }
        else if (DateTime.UtcNow - _hiddenAt > ReopenGuard)
        {
            Show();
        }
    }

    public void Show()
    {
        ResultText.Text = "";
        PlaceAboveTaskbar();
        AppWindow.Show();
        Activate();
        SetVisible(true);
        Root.Opacity = 1;
        Root.Translation = Vector3.Zero;
    }

    public void Hide()
    {
        if (!IsShown)
        {
            return;
        }
        _hiddenAt = DateTime.UtcNow;
        AppWindow.Hide();
        SetVisible(false);
        ResetEntrance();
    }

    /// <summary>托盘代理要弹出右键菜单：先交出前台再收起，菜单才能在点别处时消失。</summary>
    public void Dismiss()
    {
        if (IsShown)
        {
            ForegroundHandoff.Allow();
            Hide();
        }
    }

    /// <summary>入场动画的起点：下移、透明。下次显示时过渡到原位。</summary>
    private void ResetEntrance()
    {
        Root.Opacity = 0;
        Root.Translation = new Vector3(0, 16, 0);
    }

    private void ConfigureChrome()
    {
        AppWindow.IsShownInSwitchers = false;
        if (AppWindow.Presenter is OverlappedPresenter p)
        {
            p.IsResizable = false;
            p.IsMaximizable = false;
            p.IsMinimizable = false;
            p.IsAlwaysOnTop = true;
            // 保留边框（Windows 11 的圆角与描边），去掉标题栏。
            p.SetBorderAndTitleBar(hasBorder: true, hasTitleBar: false);
        }
    }

    /// <summary>放在主屏工作区的右下角（任务栏在其他边时也贴着那条边）。</summary>
    private void PlaceAboveTaskbar()
    {
        var display = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary);
        var work = display.WorkArea;
        var outer = display.OuterBounds;
        var scale = Content.XamlRoot?.RasterizationScale ?? this.Scale();
        var height = Root.ActualHeight > 0 ? Root.ActualHeight : FallbackHeight;
        var frameW = AppWindow.Size.Width - AppWindow.ClientSize.Width;
        var frameH = AppWindow.Size.Height - AppWindow.ClientSize.Height;
        AppWindow.Resize(new SizeInt32((int)(Width * scale) + frameW, (int)Math.Ceiling(height * scale) + frameH));
        var w = AppWindow.Size.Width;
        var h = AppWindow.Size.Height;
        var gap = (int)(EdgeGap * scale);

        var x = work.X + work.Width - w - gap;
        var y = work.Y + work.Height - h - gap;
        if (work.Y > outer.Y)
        {
            y = work.Y + gap; // 任务栏在顶部
        }
        if (work.X > outer.X)
        {
            x = work.X + gap; // 任务栏在左侧
        }
        AppWindow.Move(new PointInt32(x, y));
    }

    private void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        if (args.WindowActivationState == WindowActivationState.Deactivated)
        {
            Hide();
        }
    }

    private void SetVisible(bool visible) => _host.Presence.SetFlyoutVisible(visible);

    private void Render(HomeView v)
    {
        Dot.Show(v.Level);
        if (_shownTitle is not null && _shownTitle != v.Title && IsShown)
        {
            _ = Motion.SwapAsync(StatusText, () => StatusTitle.Text = v.Title);
        }
        else
        {
            StatusTitle.Text = v.Title;
        }
        _shownTitle = v.Title;
        StatusDescription.Text = v.Description;
        Modes.Show(v.Selected, v.Effective);
        Modes.IsEnabled = CleanButton.IsEnabled = v.Connected;
    }

    private void Localize()
    {
        CleanButton.Label = Loc.Get("home.clean");
        CleanButton.BusyLabel = Loc.Get("home.cleaning");
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        _vm.Updated -= Render;
        _vm.Dispose();
        Loc.Changed -= Localize;
        _host.Theme.Changed -= ApplyTheme;
        SetVisible(false);
    }

    private void ApplyTheme()
    {
        if (Content is FrameworkElement root)
        {
            root.RequestedTheme = _host.Theme.ElementTheme;
        }
    }

    private async Task CleanAsync()
    {
        var feedback = await _vm.CleanAsync();
        ResultText.Text = feedback.Message;
    }

    private void OnOpenMain(object sender, RoutedEventArgs e)
    {
        Hide();
        OpenMainRequested?.Invoke();
    }
}
