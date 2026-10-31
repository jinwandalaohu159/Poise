using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 系统优化页：顶部进度与一键优化，下方按分组列出每一项。
/// 修改后重新读取整张列表校正（电源服务忙时要好几秒）；连着改几项时读取经过 <see cref="RefreshGate"/> 合并：
/// 不每改一项排一次，修改之前发出的读取结果也不显示（开关不会跳回去又跳回来）。
/// </summary>
public sealed partial class OptimizeViewModel(AppHost host) : ObservableObject
{
    /// <summary>分组顺序：先固定项，再逐个开关的项。</summary>
    private static readonly (string Group, bool Toggle)[] Groups =
    [
        ("power", false), ("explorer", false), ("startup_background", false), ("memory", false),
        ("context_menu", true), ("navigation_pane", true), ("startup_apps", true),
    ];

    private readonly Dictionary<string, OptimizationRow> _rows = [];
    private readonly RefreshGate _gate = new();

    public ObservableCollection<OptimizationSection> Sections { get; } = [];

    [ObservableProperty]
    public partial bool IsLoading { get; set; }

    /// <summary>推荐项的优化进度（页面顶部）。</summary>
    public event Action<OptimizeSummary>? SummaryChanged;

    public event Action<string>? Error;

    public async Task LoadAsync()
    {
        IsLoading = true;
        try
        {
            var items = await host.Optimizations.ReloadAsync();
            SummaryChanged?.Invoke(OptimizeSummary.From(items));
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

    internal Task ApplyAsync(OptimizationRow row) => Change(row, true, () => host.Api.ApplyOptimizationAsync(row.Id));

    internal Task RevertAsync(OptimizationRow row) => Change(row, false, () => host.Api.RevertOptimizationAsync(row.Id));

    /// <summary>
    /// 点下即显示"正在处理"；服务确认后立即切到新状态，随后在后台重新读取整张列表校正（读取较慢，不让用户等它）。
    /// </summary>
    private async Task Change(OptimizationRow row, bool optimize, Func<Task> action)
    {
        row.SetBusy(true);
        try
        {
            await _gate.ChangeAsync(action);
            row.Assume(optimize);
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

    /// <summary>一键优化：只处理推荐项。</summary>
    public async Task<Feedback> OptimizeAllAsync()
    {
        try
        {
            ApplyReport report = null!;
            await _gate.ChangeAsync(async () => report = await host.Api.ApplyRecommendedAsync());
            await RefreshStatesAsync();
            return new Feedback(FeedbackKind.Success,
                report.Applied.Count > 0 ? Loc.Format("optimize.done", report.Applied.Count) : Loc.Get("optimize.nothing"), []);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return Feedback.Error(ErrorText.For(e));
        }
    }

    public async Task RevertAllAsync()
    {
        try
        {
            await _gate.ChangeAsync(host.Api.RevertAllAsync);
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
            await _gate.RefreshAsync(QueryStatesAsync);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            Error?.Invoke(ErrorText.For(e));
        }
    }

    private async Task<Action> QueryStatesAsync()
    {
        var items = await host.Optimizations.ReloadAsync();
        return () =>
        {
            SummaryChanged?.Invoke(OptimizeSummary.From(items));
            foreach (var status in items)
            {
                if (_rows.TryGetValue(status.Item.Id, out var row))
                {
                    row.Update(status);
                }
            }
        };
    }
}
