using System.Numerics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Poise.Core.Localization;
using Poise.App.UI.Theme;
using Poise.Core.Presentation;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Windows;

/// <summary>
/// 托盘面板：四个指标、模式、一键清理、打开主窗口。贴着任务栏弹出，点别处就收起。
/// 只创建一次，之后显示/隐藏复用。点托盘图标时由托盘代理唤起（见 App）。
/// </summary>
public sealed partial class TrayFlyoutWindow : Window
{
    private const int Width = 360;
    /// <summary>首次显示前还没有布局结果时使用的高度。</summary>
    private const int FallbackHeight = 280;
    /// <summary>点托盘图标会先让面板失去焦点而收起，紧接着的点击不应再把它打开。</summary>
    private static readonly TimeSpan ReopenGuard = TimeSpan.FromMilliseconds(300);

    private readonly AppHost _host;
    private readonly HomeViewModel _vm;
    private readonly ForegroundWatcher _foreground = new();
    private readonly IntPtr _hwnd;
    private DateTime _hiddenAt = DateTime.MinValue;

    public TrayFlyoutWindow(AppHost host)
    {
        _host = host;
        _vm = host.Home;
        InitializeComponent();
        Title = "Poise";
        _hwnd = WinRT.Interop.WindowNative.GetWindowHandle(this);
        SystemBackdrop = new DesktopAcrylicBackdrop();
        FlyoutPlacement.ConfigureChrome(AppWindow);

        Modes.Compact = true;
        Modes.ModeChosen += mode => _ = _vm.SetModeAsync(mode);
        CleanButton.IsAccent = true;
        CleanButton.Glyph = "\uE74D";
        CleanButton.Action = CleanAsync;
        Localize();
        Motion.EnableTransitions(Root);
        ResetEntrance();
        // 窗口高度跟随内容：语言、描述换行变化时自动校正。
        Root.SizeChanged += (_, e) =>
        {
            if (Math.Abs(e.NewSize.Height - e.PreviousSize.Height) > 0.5)
            {
                PlaceAboveTaskbar();
            }
        };

        Loc.Changed += Localize;
        host.Theme.Changed += ApplyTheme;
        Activated += OnActivated;
        _foreground.Changed += OnForegroundChanged;
        Closed += OnClosed;
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

    /// <summary>显示面板。只在显示期间跟着状态更新（收起时不重画、不跑数字动画）。</summary>
    public void Show()
    {
        _vm.Updated -= Render;
        _vm.Updated += Render;
        if (_vm.Ready)
        {
            Render(_vm.Current);
        }
        ShowResult("");
        PlaceAboveTaskbar();
        AppWindow.Show();
        Activate();
        Foreground.Take(_hwnd);
        _foreground.Start();
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
        _vm.Updated -= Render;
        _foreground.Stop();
        AppWindow.Hide();
        SetVisible(false);
        ResetEntrance();
    }

    /// <summary>托盘代理要弹出右键菜单：先交出前台再收起，菜单才能在点别处时消失。</summary>
    public void Dismiss()
    {
        if (IsShown)
        {
            Foreground.AllowOthers();
            Hide();
        }
    }

    /// <summary>入场动画的起点：下移、透明。下次显示时过渡到原位。</summary>
    private void ResetEntrance()
    {
        Root.Opacity = 0;
        Root.Translation = new Vector3(0, 16, 0);
    }

    /// <summary>按内容高度调整窗口并贴着任务栏放好。</summary>
    private void PlaceAboveTaskbar()
    {
        var scale = Content.XamlRoot?.RasterizationScale ?? this.Scale();
        // 按当前内容量一次高度（清理结果行可能刚显示或收起），避免窗口出现后再改尺寸、跳一下。
        Root.Measure(new global::Windows.Foundation.Size(Width, double.PositiveInfinity));
        var height = Root.DesiredSize.Height > 0 ? Root.DesiredSize.Height
            : Root.ActualHeight > 0 ? Root.ActualHeight : FallbackHeight;
        FlyoutPlacement.PlaceNearTaskbar(AppWindow, Width, height, scale);
    }

    private void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        if (args.WindowActivationState == WindowActivationState.Deactivated)
        {
            Hide();
        }
    }

    /// <summary>点了别处（桌面、任务栏、其他窗口）：前台已经不是面板，收起。</summary>
    private void OnForegroundChanged(IntPtr hwnd)
    {
        if (hwnd != _hwnd)
        {
            Hide();
        }
    }

    private void SetVisible(bool visible) => _host.Presence.SetFlyoutVisible(visible);

    private void Render(HomeView v)
    {
        Metrics.Show(v);
        Modes.Show(v.Selected, v.Effective, v.InUse, v.VendorMode);
        Modes.IsEnabled = CleanButton.IsEnabled = v.Connected;
    }

    private void Localize()
    {
        CleanButton.Label = Loc.Get("home.clean");
        CleanButton.BusyLabel = Loc.Get("home.cleaning");
        ToolTipService.SetToolTip(OpenButton, Loc.Get("tray.open"));
        AutomationProperties.SetName(OpenButton, Loc.Get("tray.open"));
    }

    private void OnClosed(object sender, WindowEventArgs args)
    {
        _vm.Updated -= Render;
        _foreground.Dispose();
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
        ShowResult(feedback.Message);
    }

    /// <summary>清理结果：有内容时才占一行，面板高度随之调整。</summary>
    private void ShowResult(string text)
    {
        ResultText.Text = text;
        ResultText.Visibility = text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnOpenMain(object sender, RoutedEventArgs e)
    {
        Hide();
        OpenMainRequested?.Invoke();
    }
}
