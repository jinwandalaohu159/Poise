using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml.Media;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.Services;

namespace Poise.App.UI.ViewModels;

/// <summary>程序列表中的一行。<see cref="PreferenceIndex"/> 即 <see cref="AppPreference"/> 的数值。</summary>
public sealed partial class AppItem : ObservableObject
{
    private readonly AppsViewModel _owner;
    private bool _syncing;

    public AppItem(AppsViewModel owner, AppEntry entry)
    {
        _owner = owner;
        Name = entry.Name;
        Path = entry.Path;
        Running = entry.Running;
        _syncing = true;
        PreferenceIndex = (int)entry.Preference;
        _syncing = false;
    }

    public string Name { get; }

    public string Path { get; }

    public bool Running { get; }

    [ObservableProperty]
    public partial int PreferenceIndex { get; set; }

    [ObservableProperty]
    public partial ImageSource? Icon { get; set; }

    partial void OnPreferenceIndexChanged(int oldValue, int newValue)
    {
        if (_syncing || newValue < 0)
        {
            return;
        }
        _ = _owner.SetPreferenceAsync(this, (AppPreference)newValue, (AppPreference)oldValue);
    }

    /// <summary>服务拒绝时回到原来的选项，不再触发请求。</summary>
    internal void Restore(AppPreference value)
    {
        _syncing = true;
        PreferenceIndex = (int)value;
        _syncing = false;
    }
}

/// <summary>程序偏好页：正在运行的程序 + 设置过偏好的程序，可搜索。</summary>
public sealed partial class AppsViewModel(AppHost host) : ObservableObject
{
    private List<AppItem> _all = [];

    public ObservableCollection<AppItem> Running { get; } = [];

    public ObservableCollection<AppItem> Configured { get; } = [];

    [ObservableProperty]
    public partial string Query { get; set; } = "";

    [ObservableProperty]
    public partial bool IsEmpty { get; set; }

    /// <summary>出错时的提示（显示在页面顶部）。</summary>
    public event Action<string>? Error;

    partial void OnQueryChanged(string value) => ApplyFilter();

    public async Task LoadAsync()
    {
        try
        {
            var entries = await host.Api.ListAppsAsync();
            _all = entries.Select(e => new AppItem(this, e)).ToList();
            ApplyFilter();
            foreach (var item in _all)
            {
                item.Icon = await AppIcons.LoadAsync(item.Path);
            }
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
    }

    private void ApplyFilter()
    {
        var q = Query.Trim();
        var visible = _all.Where(a => q.Length == 0 || a.Name.Contains(q, StringComparison.CurrentCultureIgnoreCase)).ToList();
        Replace(Running, visible.Where(a => a.Running));
        Replace(Configured, visible.Where(a => !a.Running));
        IsEmpty = visible.Count == 0;
    }

    private static void Replace(ObservableCollection<AppItem> target, IEnumerable<AppItem> items)
    {
        target.Clear();
        foreach (var i in items)
        {
            target.Add(i);
        }
    }

    internal async Task SetPreferenceAsync(AppItem item, AppPreference value, AppPreference previous)
    {
        try
        {
            await host.Api.SetAppPreferenceAsync(item.Path, value);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            item.Restore(previous);
            Error?.Invoke(ErrorText.For(e));
        }
    }
}
