using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 一个优化项。两种呈现：
/// 固定项（资源管理器、启动、内存）显示状态和"优化 / 还原"按钮；
/// 逐个开关的项（右键插件、导航栏、启动项）显示开关，开 = 保持原样，关 = 禁用。
/// </summary>
public sealed partial class OptimizationRow : ObservableObject
{
    private readonly OptimizeViewModel _owner;

    public OptimizationRow(OptimizeViewModel owner, ItemStatus status, bool isToggle)
    {
        _owner = owner;
        Id = status.Item.Id;
        Title = Resolve(status.Item.Title);
        Description = Resolve(status.Item.Description);
        EffectHint = Loc.Get($"effect.{status.Item.TakesEffect}");
        SlowHint = status.Item.SlowHint;
        IsToggle = isToggle;
        Update(status);
    }

    public string Id { get; }

    public string Title { get; }

    public string Description { get; }

    public string EffectHint { get; }

    public bool SlowHint { get; }

    public bool IsToggle { get; }

    [ObservableProperty]
    public partial bool IsOptimized { get; private set; }

    [ObservableProperty]
    public partial bool Revertible { get; private set; }

    [ObservableProperty]
    public partial bool IsBusy { get; private set; }

    /// <summary>开关：开 = 保持原样（未优化）。</summary>
    public bool IsOn
    {
        get => !IsOptimized;
        set
        {
            if (value == !IsOptimized || IsBusy)
            {
                return;
            }
            _ = value ? _owner.RevertAsync(this) : _owner.ApplyAsync(this);
        }
    }

    /// <summary>能否操作：未优化的可以优化；已优化的只有 Poise 自己改的才能还原。</summary>
    public bool CanChange => !IsBusy && (!IsOptimized || Revertible);

    public string StateText => Loc.Get(IsOptimized ? "optimize.state.on" : "optimize.state.off");

    public string ActionText => !IsOptimized ? Loc.Get("optimize.apply") : Revertible ? Loc.Get("common.revert") : "";

    public bool HasAction => ActionText.Length > 0;

    public bool HasEffectHint => EffectHint.Length > 0;

    internal void SetBusy(bool busy)
    {
        IsBusy = busy;
        OnPropertyChanged(nameof(CanChange));
    }

    internal void Update(ItemStatus status)
    {
        IsOptimized = status.IsOptimized;
        Revertible = status.Revertible;
        OnPropertyChanged(nameof(IsOn));
        OnPropertyChanged(nameof(CanChange));
        OnPropertyChanged(nameof(StateText));
        OnPropertyChanged(nameof(ActionText));
        OnPropertyChanged(nameof(HasAction));
    }

    /// <summary>"优化 / 还原"按钮。</summary>
    public void Invoke() => _ = IsOptimized ? _owner.RevertAsync(this) : _owner.ApplyAsync(this);

    private static string Resolve(TextValue text) => text.Type == "key" ? Loc.Get(text.Value) : text.Value;
}

public sealed record OptimizationSection(string Title, string? Hint, bool IsToggle, ObservableCollection<OptimizationRow> Rows);

/// <summary>优化记录页。</summary>
public sealed partial class OptimizeViewModel(AppHost host) : ObservableObject
{
    /// <summary>分组顺序：先固定项，再逐个开关的项。</summary>
    private static readonly (string Group, bool Toggle)[] Groups =
    [
        ("explorer", false), ("startup_background", false), ("memory", false),
        ("context_menu", true), ("navigation_pane", true), ("startup_apps", true),
    ];

    private readonly Dictionary<string, OptimizationRow> _rows = [];

    public ObservableCollection<OptimizationSection> Sections { get; } = [];

    [ObservableProperty]
    public partial bool IsLoading { get; set; }

    public event Action<string>? Error;

    public async Task LoadAsync()
    {
        IsLoading = true;
        try
        {
            var items = await host.Api.ListOptimizationsAsync();
            Sections.Clear();
            _rows.Clear();
            foreach (var (group, toggle) in Groups)
            {
                var rows = items.Where(i => i.Item.Group == group)
                    .Select(i => _rows[i.Item.Id] = new OptimizationRow(this, i, toggle))
                    .ToList();
                if (rows.Count > 0)
                {
                    var hint = group == "context_menu" ? Loc.Get("optimize.plugins_hint") : null;
                    Sections.Add(new OptimizationSection(Loc.Get($"group.{group}"), hint, toggle, new(rows)));
                }
            }
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
        finally
        {
            IsLoading = false;
        }
    }

    internal Task ApplyAsync(OptimizationRow row) => Change(row, () => host.Api.ApplyOptimizationAsync(row.Id));

    internal Task RevertAsync(OptimizationRow row) => Change(row, () => host.Api.RevertOptimizationAsync(row.Id));

    private async Task Change(OptimizationRow row, Func<Task> action)
    {
        row.SetBusy(true);
        try
        {
            await action();
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
        finally
        {
            row.SetBusy(false);
        }
        await RefreshStatesAsync();
    }

    public async Task RevertAllAsync()
    {
        try
        {
            await host.Api.RevertAllAsync();
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
        await RefreshStatesAsync();
    }

    /// <summary>只刷新每行的状态，不重建列表（避免整页闪动）。</summary>
    private async Task RefreshStatesAsync()
    {
        try
        {
            foreach (var status in await host.Api.ListOptimizationsAsync())
            {
                if (_rows.TryGetValue(status.Item.Id, out var row))
                {
                    row.Update(status);
                }
            }
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
    }
}
