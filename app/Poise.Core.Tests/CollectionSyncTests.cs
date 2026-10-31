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
    public void Resorting_most_of_a_long_list_resets_it_once_with_the_same_objects()
    {
        var items = Enumerable.Range(1, 10).Select(i => new Item(i, "")).ToList();
        var target = new ResettableCollection<Item>();
        items.ForEach(target.Add);
        var changes = new List<NotifyCollectionChangedAction>();
        target.CollectionChanged += (_, e) => changes.Add(e.Action);
        var reversed = Enumerable.Range(1, 10).Reverse().Select(i => new Item(i, "")).ToList();

        CollectionSync.Sync(target, reversed, i => i.Id);

        Assert.Equal(Enumerable.Range(1, 10).Reverse(), target.Select(i => i.Id));
        Assert.Same(items[0], target[9]);
        Assert.Equal([NotifyCollectionChangedAction.Reset], changes);
    }

    [Fact]
    public void A_few_moves_in_a_long_list_stay_moves()
    {
        var items = Enumerable.Range(1, 10).Select(i => new Item(i, "")).ToList();
        var target = new ResettableCollection<Item>();
        items.ForEach(target.Add);
        var changes = new List<NotifyCollectionChangedAction>();
        target.CollectionChanged += (_, e) => changes.Add(e.Action);

        CollectionSync.Sync(target, [items[1], items[0], .. items[2..]], i => i.Id);

        Assert.Equal([NotifyCollectionChangedAction.Move], changes);
    }

    [Fact]
    public void A_row_added_at_the_top_of_a_long_list_is_one_insert()
    {
        var items = Enumerable.Range(1, 10).Select(i => new Item(i, "")).ToList();
        var target = new ResettableCollection<Item>();
        items.ForEach(target.Add);
        var changes = new List<NotifyCollectionChangedAction>();
        target.CollectionChanged += (_, e) => changes.Add(e.Action);

        // 下面的行位置号都变了，相对顺序没变：只插一行，不整表重置。
        CollectionSync.Sync(target, [new Item(0, ""), .. items], i => i.Id);

        Assert.Equal([NotifyCollectionChangedAction.Add], changes);
        Assert.Equal(Enumerable.Range(0, 11), target.Select(i => i.Id));
    }

    [Fact]
    public void Misplaced_rows_follow_relative_order()
    {
        int[] current = [1, 2, 3, 4, 5];
        Assert.Equal(0, CollectionSync.Misplaced(current, [9, 1, 2, 3, 4, 5], i => i));
        Assert.Equal(1, CollectionSync.Misplaced(current, [2, 3, 4, 5, 1], i => i));
        Assert.Equal(4, CollectionSync.Misplaced(current, [5, 4, 3, 2, 1], i => i));
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
