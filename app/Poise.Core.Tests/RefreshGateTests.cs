using Poise.Core.Monitoring;

namespace Poise.Core.Tests;

/// <summary>定期查询与用户修改交错时，界面不能先显示旧状态再跳回来。</summary>
public sealed class RefreshGateTests
{
    private readonly RefreshGate _gate = new();
    private readonly List<int> _applied = [];
    private readonly Queue<Task<int>> _answers = new();

    private Task Refresh() => _gate.RefreshAsync(async () =>
    {
        var value = await _answers.Dequeue();
        return () => _applied.Add(value);
    });

    [Fact]
    public async Task A_result_that_was_in_flight_during_a_change_is_dropped_and_queried_again()
    {
        var stale = new TaskCompletionSource<int>();
        _answers.Enqueue(stale.Task);
        _answers.Enqueue(Task.FromResult(2));

        var refresh = Refresh();
        await _gate.ChangeAsync(() => Task.CompletedTask);
        stale.SetResult(1);
        await refresh;

        Assert.Equal([2], _applied);
    }

    [Fact]
    public async Task Results_are_held_back_while_a_change_is_running()
    {
        var change = new TaskCompletionSource();
        _answers.Enqueue(Task.FromResult(1));

        var changing = _gate.ChangeAsync(() => change.Task);
        await Refresh();
        Assert.Empty(_applied);

        change.SetResult();
        await changing;
        _answers.Enqueue(Task.FromResult(2));
        await Refresh();
        Assert.Equal([2], _applied);
    }

    [Fact]
    public async Task A_refresh_requested_while_busy_runs_once_more_afterwards()
    {
        var first = new TaskCompletionSource<int>();
        _answers.Enqueue(first.Task);
        _answers.Enqueue(Task.FromResult(2));

        var running = Refresh();
        await Refresh();
        first.SetResult(1);
        await running;

        Assert.Equal([1, 2], _applied);
    }
}
