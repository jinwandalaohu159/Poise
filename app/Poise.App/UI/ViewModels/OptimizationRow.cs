using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Ipc;
using Poise.Core.Localization;

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

    public string StateText => Loc.Get(IsBusy ? "optimize.state.busy" : IsOptimized ? "optimize.state.on" : "optimize.state.off");

    public string ActionText => !IsOptimized ? Loc.Get("optimize.apply") : Revertible ? Loc.Get("common.revert") : "";

    public bool HasAction => ActionText.Length > 0;

    /// <summary>圆圈问号里的说明：作用，以及什么时候生效。</summary>
    public string Help => string.Join(Environment.NewLine, new[] { Description, EffectHint }.Where(t => t.Length > 0));

    internal void SetBusy(bool busy)
    {
        IsBusy = busy;
        OnPropertyChanged(nameof(CanChange));
        OnPropertyChanged(nameof(StateText));
    }

    internal void Update(ItemStatus status) => Show(status.IsOptimized, status.Revertible);

    /// <summary>服务确认操作成功后立即显示结果，不等整张列表重新读取。</summary>
    internal void Assume(bool optimized) => Show(optimized, optimized || Revertible);

    private void Show(bool optimized, bool revertible)
    {
        IsOptimized = optimized;
        Revertible = revertible;
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

public sealed record OptimizationSection(string Title, string? Hint, bool IsToggle, ObservableCollection<OptimizationRow> Rows)
{
    /// <summary>分组标题旁圆圈问号的说明；没有时为空（不显示问号）。</summary>
    public string HintText => Hint ?? "";
}
