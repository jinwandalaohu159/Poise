using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Poise.Core.Localization;
using Poise.App.UI.Pages;
using Windows.Graphics;

namespace Poise.App.UI.Windows;

/// <summary>主窗口：Mica 背景、扩展标题栏、页面跳转。关闭后释放，托盘继续运行。</summary>
public sealed partial class MainWindow : Window
{
    private const int DefaultWidth = 760;
    private const int DefaultHeight = 620;
    private const int MinWidth = 560;
    private const int MinHeight = 520;

    private readonly AppHost _host;

    public MainWindow(AppHost host)
    {
        _host = host;
        InitializeComponent();
        Title = "Poise";
        SystemBackdrop = new MicaBackdrop();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Poise.ico"));
        ToolTipService.SetToolTip(SettingsButton, Loc.Get("settings.title"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(SettingsButton, Loc.Get("settings.title"));
        SizeWindow();

        Root.Loaded += (_, _) =>
        {
            _host.Theme.Apply(this, animate: false);
            UpdatePassthrough();
        };
        Root.SizeChanged += (_, _) => UpdatePassthrough();
        _host.Theme.Changed += OnThemeChanged;
        Loc.Changed += OnLanguageChanged;
        Activated += (_, _) => SetVisible(!IsMinimized);
        AppWindow.Changed += (_, args) =>
        {
            if (args.DidPresenterChange || args.DidSizeChange)
            {
                SetVisible(!IsMinimized);
            }
        };
        Closed += OnClosed;

        ContentFrame.Navigate(typeof(HomePage), null, new SuppressNavigationTransitionInfo());
    }

    /// <summary>窗口已经打开时再次请求打开：恢复并置前。</summary>
    public void Present()
    {
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } p)
        {
            p.Restore();
        }
        Activate();
    }

    private bool IsMinimized =>
        AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized };

    public void Navigate(Type page)
    {
        if (ContentFrame.CurrentSourcePageType != page)
        {
            ContentFrame.Navigate(page, null, new DrillInNavigationTransitionInfo());
        }
    }

    private void OnSettingsClick(object sender, RoutedEventArgs e) => Navigate(typeof(SettingsPage));

    private void SizeWindow()
    {
        var scale = this.Scale();
        AppWindow.Resize(new SizeInt32((int)(DefaultWidth * scale), (int)(DefaultHeight * scale)));
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = (int)(MinWidth * scale);
            presenter.PreferredMinimumHeight = (int)(MinHeight * scale);
        }
        var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
        AppWindow.Move(new PointInt32(
            area.X + (area.Width - AppWindow.Size.Width) / 2,
            area.Y + (area.Height - AppWindow.Size.Height) / 2));
    }

    /// <summary>标题栏区域默认用于拖动窗口；设置按钮需要单独声明为可点击区域。</summary>
    private void UpdatePassthrough()
    {
        if (Root.XamlRoot is null)
        {
            return;
        }
        var scale = Root.XamlRoot.RasterizationScale;
        var bounds = SettingsButton.TransformToVisual(Root).TransformBounds(
            new global::Windows.Foundation.Rect(0, 0, SettingsButton.ActualWidth, SettingsButton.ActualHeight));
        var rect = new RectInt32((int)(bounds.X * scale), (int)(bounds.Y * scale), (int)(bounds.Width * scale), (int)(bounds.Height * scale));
        InputNonClientPointerSource.GetForWindowId(AppWindow.Id).SetRegionRects(NonClientRegionKind.Passthrough, [rect]);
    }

    private void OnThemeChanged() => _host.Theme.Apply(this, animate: true);

    /// <summary>切换语言后立即生效：重建当前页面。</summary>
    private void OnLanguageChanged()
    {
        ToolTipService.SetToolTip(SettingsButton, Loc.Get("settings.title"));
        if (ContentFrame.CurrentSourcePageType is { } page)
        {
            ContentFrame.Navigate(page, null, new SuppressNavigationTransitionInfo());
            ContentFrame.BackStack.RemoveAt(ContentFrame.BackStack.Count - 1);
        }
    }

    private void SetVisible(bool visible) => _host.Presence.SetMainVisible(visible);

    private void OnClosed(object sender, WindowEventArgs args)
    {
        _host.Theme.Changed -= OnThemeChanged;
        Loc.Changed -= OnLanguageChanged;
        SetVisible(false);
        // 释放页面，让页面里的 ViewModel 取消订阅。
        ContentFrame.Content = null;
    }
}
