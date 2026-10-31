using Poise.Core.Ipc;

namespace Poise.App.UI.Services;

/// <summary>
/// 系统优化项的现状（主页的优化进度与优化页共用）。列一次要服务重新枚举右键菜单插件、读一批注册表和
/// 电源设置，所以主页每次显示时用一分钟内读过的结果；优化页打开、改过之后重新读。只在界面线程上使用。
/// </summary>
public sealed class OptimizationStates(PoiseApi api)
{
    private static readonly TimeSpan FreshFor = TimeSpan.FromMinutes(1);

    private List<ItemStatus>? _items;
    private DateTime _at;

    /// <summary>最近读过的（一分钟内），否则重新读。</summary>
    public Task<List<ItemStatus>> GetAsync() =>
        _items is { } items && DateTime.UtcNow - _at < FreshFor ? Task.FromResult(items) : ReloadAsync();

    /// <summary>重新读。</summary>
    public async Task<List<ItemStatus>> ReloadAsync()
    {
        var items = await api.ListOptimizationsAsync();
        _items = items;
        _at = DateTime.UtcNow;
        return items;
    }
}
