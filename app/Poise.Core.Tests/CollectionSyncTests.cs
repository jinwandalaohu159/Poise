using System.Collections.ObjectModel;
using System.Collections.Specialized;
using Poise.Core.Collections;

namespace Poise.Core.Tests;

public sealed class CollectionSyncTests
{
    private sealed record Item(int Id, string Text);

    [Fact]
    public void Keeps_existing_objects_and_only_touches_what_changed()
    {
        var a = new Item(1, "a");
        var b = new Item(2, "b");
        var c = new Item(3, "c");
        var target = new ObservableCollection<Item> { a, b, c };
        var changes = new List<NotifyCollectionChangedAction>();
        target.CollectionChanged += (_, e) => changes.Add(e.Action);

        var d = new Item(4, "d");
        CollectionSync.Sync(target, [a, d, c], i => i.Id);

        Assert.Equal([a, d, c], target);
        Assert.Same(a, target[0]);
        Assert.Equal([NotifyCollectionChangedAction.Remove, NotifyCollectionChangedAction.Add], changes);
    }

    [Fact]
    public void Reorders_with_moves_instead_of_rebuilding()
    {
        var items = Enumerable.Range(1, 4).Select(i => new Item(i, "")).ToList();
        var target = new ObservableCollection<Item>(items);
        var changes = new List<NotifyCollectionChangedAction>();
        target.CollectionChanged += (_, e) => changes.Add(e.Action);

        CollectionSync.Sync(target, [items[3], items[0], items[1], items[2]], i => i.Id);

        Assert.Equal([4, 1, 2, 3], target.Select(i => i.Id));
        Assert.Equal([NotifyCollectionChangedAction.Move], changes);
    }

    [Fact]
    public void Handles_empty_lists()
    {
        var target = new ObservableCollection<Item> { new(1, "") };

        CollectionSync.Sync(target, [], i => i.Id);
        Assert.Empty(target);

        CollectionSync.Sync(target, [new(2, "")], i => i.Id);
        Assert.Equal([2], target.Select(i => i.Id));
    }
}
