using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.App.UI.Controls;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.Core.Settings;
using Poise.App.UI.ViewModels;
using Windows.Storage.Pickers;

namespace Poise.App.UI.Pages;

/// <summary>
/// 程序偏好页。列表、分栏、智能调度开关和定期刷新都在 <see cref="AppsViewModel"/> 里，
/// 这里只管页面的显示与隐藏、列名的排序状态，以及"添加程序"的文件选择和对话框。
/// </summary>
public sealed partial class AppsPage : Page
{
    /// <summary>对话框里的选项顺序与 <see cref="AppPreference"/> 一致。</summary>
    private static readonly string[] PreferenceKeys = ["pref.auto", "pref.always_fast", "pref.background", "pref.game"];

    public AppsPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("apps.title");
        Vm.Error += ErrorBar.ShowError;
        Vm.Recovered += ErrorBar.HideError;
        NameHeader.Click += () => Vm.SortBy(AppSort.Name);
        CpuHeader.Click += () => Vm.SortBy(AppSort.Cpu);
        MemoryHeader.Click += () => Vm.SortBy(AppSort.Memory);
        Vm.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(AppsViewModel.Sort))
            {
                ShowSort();
            }
        };
        ShowSort();
    }

    public AppsViewModel Vm { get; } = new(AppHost.Current);

    protected override void OnNavigatedTo(NavigationEventArgs e) => Vm.Start();

    protected override void OnNavigatedFrom(NavigationEventArgs e) => Vm.Stop();

    /// <summary>选择一个程序（.exe），再选择偏好。</summary>
    /// <summary>当前排序的列加深并显示方向箭头；每个列名的提示说明再点会怎样排序。</summary>
    private void ShowSort()
    {
        var sort = Vm.Sort;
        foreach (var (header, column) in new[] { (NameHeader, AppSort.Name), (CpuHeader, AppSort.Cpu), (MemoryHeader, AppSort.Memory) })
        {
            header.IsActive = sort.Column == column;
            header.Descending = sort.Column == column && sort.Descending;
            header.Hint = AppsText.SortHint(sort, column);
        }
    }

    private async void OnAdd(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker { SuggestedStartLocation = PickerLocationId.ComputerFolder };
        picker.FileTypeFilter.Add(".exe");
        WinRT.Interop.InitializeWithWindow.Initialize(picker, Win32Interop.GetWindowFromWindowId(XamlRoot.ContentIslandEnvironment.AppWindowId));
        var file = await picker.PickSingleFileAsync();
        if (file is null)
        {
            return;
        }
        var name = string.IsNullOrEmpty(file.DisplayName) ? file.Name : file.DisplayName;
        if (await ChoosePreferenceAsync(name) is { } preference)
        {
            await Vm.AddAsync(file.Path, preference);
        }
    }

    private async Task<AppPreference?> ChoosePreferenceAsync(string appName)
    {
        var options = new RadioButtons();
        foreach (var key in PreferenceKeys)
        {
            options.Items.Add(new RadioButton
            {
                Content = new StackPanel
                {
                    Children =
                    {
                        new TextBlock { Text = Loc.Get(key) },
                        new TextBlock
                        {
                            Text = Loc.Get($"{key}.desc"),
                            Style = (Style)Application.Current.Resources["CaptionSecondaryTextStyle"],
                        },
                    },
                },
            });
        }
        options.SelectedIndex = (int)AppPreference.AlwaysFast;
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = Loc.Format("apps.add.title", appName),
            Content = options,
            PrimaryButtonText = Loc.Get("apps.add.save"),
            CloseButtonText = Loc.Get("common.cancel"),
            DefaultButton = ContentDialogButton.Primary,
            RequestedTheme = ActualTheme,
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary && options.SelectedIndex >= 0
            ? (AppPreference)options.SelectedIndex
            : null;
    }
}
