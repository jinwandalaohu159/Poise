using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Controls;
using Poise.App.UI.ViewModels;
using Poise.Core.Localization;

namespace Poise.App.UI.Pages;

/// <summary>
/// 日志与记录页：日志（Poise 做过的调整和原因，可按类别筛选，见 <see cref="EventLogViewModel"/>）与
/// 使用记录（每次游戏、重活一条，见 <see cref="RecordsViewModel"/>）两个标签。只有显示着的标签在读取，
/// 这里只管标签切换和打开文件夹。
/// </summary>
public sealed partial class ActivityPage : Page
{
    private bool _shown;

    public ActivityPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("activity.title");
        Log.Error += ErrorBar.ShowError;
        Records.Error += ErrorBar.ShowError;
        Loaded += (_, _) =>
        {
            _shown = true;
            ShowTab();
        };
        Unloaded += (_, _) =>
        {
            _shown = false;
            Log.Stop();
            Records.Stop();
        };
    }

    public EventLogViewModel Log { get; } = new(AppHost.Current);

    public RecordsViewModel Records { get; } = new(AppHost.Current);

    private void OnTabChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        // 载入页面时标签可能先于下面的面板建好就触发一次；面板建好后由 Loaded 再显示。
        if (RecordsPanel is not null)
        {
            ShowTab();
        }
    }

    /// <summary>显示选中的标签，只让它读取；筛选与日志文件夹只对日志有用。</summary>
    private void ShowTab()
    {
        var log = Tabs.SelectedItem != RecordsTab;
        var logVisibility = log ? Visibility.Visible : Visibility.Collapsed;
        LogPanel.Visibility = logVisibility;
        FilterBox.Visibility = logVisibility;
        FolderLink.Visibility = logVisibility;
        RecordsPanel.Visibility = log ? Visibility.Collapsed : Visibility.Visible;
        if (!_shown)
        {
            return;
        }
        if (log)
        {
            Records.Stop();
            Log.Start();
        }
        else
        {
            Log.Stop();
            Records.Start();
        }
    }

    private void OnOpenFolder(object sender, RoutedEventArgs e) => Log.OpenFolder();
}
