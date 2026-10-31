using Poise.Core.Ipc;
using Poise.Core.Presentation;

namespace Poise.Core.Tests;

/// <summary>
/// 核心与缓存面板、日志、使用记录：服务（Rust）生成的样本能读懂，并且写成的文字正确
/// （CoresText、EventText、RecordText、CoreWording）。
/// </summary>
public sealed class CoresAndLogTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.Now;
    private static readonly Machine Hybrid = new(true, "dell_g_mode");
    private static readonly Machine TwoCcds = new(false, null);

    public CoresAndLogTests() => TestSupport.UseChinese();

    private static CoreMap Cores => TestSupport.Fixture<CoreMap>("cores.json");

    // ---- 契约 ----

    [Fact]
    public void Reads_the_core_map()
    {
        var map = Cores;

        Assert.True(map.Partitioned && map.SmartSchedule);
        Assert.Equal(Pressure.High, map.Demand);
        var big = map.Domains[0];
        Assert.Equal((true, 6u, 12u, 1280u * 1024, 2u, 24u << 20, (uint?)0), (big.Performance, big.Cores, big.Threads, big.L2Bytes, big.L2Threads, big.L3Bytes, big.L3Group));
        Assert.Equal(new ProcessorLoad(1, 0, 45f), big.Processors[1]);
        Assert.Equal(new PlacedProgram(7, "League of Legends", @"c:\games\7.exe", Verdict.Playing, 260, 2), Assert.Single(big.Programs));
        var small = map.Domains[1];
        Assert.Null(small.LoadPct);
        Assert.Equal(new ProcessorLoad(64, 256, null), Assert.Single(small.Processors));
        Assert.Equal([Verdict.Playing, Verdict.Yielded, Verdict.Aside], small.Programs.Select(p => p.Verdict));
        Assert.Equal(Verdict.Lowered, Assert.Single(map.Unrestricted).Verdict);
    }

    [Fact]
    public void Reads_every_kind_of_event()
    {
        var page = TestSupport.Fixture<EventPage>("events.json");

        Assert.Equal(@"C:\ProgramData\Poise", page.LogDir);
        Assert.Equal(Enumerable.Range(1, page.Entries.Count).Select(i => (ulong)i), page.Entries.Select(e => e.Id));
        // 每种都读成了具体的类型，没有退回基类。
        Assert.All(page.Entries, e => Assert.NotEqual(typeof(PoiseEvent), e.Event.GetType()));
        var program = Assert.IsType<ProgramEvent>(page.Entries[1].Event);
        Assert.Equal((Verdict.Normal, Verdict.Playing, new CoreSpan(false, true, 1), 260u, 3u, (Cause?)null), (program.From, program.To, program.Cores, program.LoadPct, program.DiskMbS, program.Cause));
        Assert.Equal(Cause.Foreground, Assert.IsType<ProgramEvent>(page.Entries[2].Event).Cause);
        var restored = page.Entries.Select(e => e.Event).OfType<SettingRestoredEvent>().Single();
        Assert.Equal(new SettingRestoredEvent(PowerSettingKind.MinState, 1, true, 100, 5), restored);
        var cleaned = page.Entries.Select(e => e.Event).OfType<CleanedEvent>().Single();
        Assert.Equal((CleanupTrigger.Auto, 1UL << 30, 2u, 1u), (cleaned.Trigger, cleaned.FreedBytes, cleaned.Handled, cleaned.Failed));
        Assert.Equal(EventCategory.Schedule, page.Entries[0].Category);
        Assert.Equal(new DemandEvent(Pressure.High, DemandReason.Focus), page.Entries[0].Event);
        Assert.Equal("有重点程序", EventText.Row(page.Entries[0], Hybrid, Now).Detail);
        var lookalike = Assert.IsType<ProgramEvent>(page.Entries[^1].Event);
        Assert.Equal((Verdict.Focused, (Cause?)Cause.LooksLikeGame), (lookalike.To, lookalike.Cause));
        Assert.Contains("像游戏，等待确认", EventText.Row(page.Entries[^1], Hybrid, Now).Detail);
        Assert.Contains(page.Entries, e => e.Event is UnthrottledEvent && e.Category == EventCategory.Mode);
    }

    [Fact]
    public void Unknown_event_kinds_from_a_newer_service_are_still_read()
    {
        var json = """{"entries":[{"id":1,"at_unix_ms":0,"category":"system","event":{"type":"from_the_future","x":1}}],"log_dir":""}""";

        var page = System.Text.Json.JsonSerializer.Deserialize<EventPage>(json, ProtocolJson.Options)!;

        Assert.Equal(typeof(PoiseEvent), Assert.Single(page.Entries).Event.GetType());
        Assert.Equal("其他事件", EventText.Row(page.Entries[0], Hybrid, Now).Title);
    }

    [Fact]
    public void Reads_usage_records()
    {
        var records = TestSupport.Fixture<List<SessionRecord>>("records.json");

        var game = records[0];
        Assert.Equal([Mode.Vendor, Mode.Full], game.Modes);
        Assert.Equal(new RecordProgram("League of Legends", @"c:\riot games\league of legends.exe", true, 2350, 310, 0b011), game.Programs[0]);
        Assert.Null(game.Programs[1].Path);
        Assert.Equal((23u, 4u, 86.5f, 99f, 180u), (game.MovedAsidePeak, game.YieldedPrograms, game.CpuTempAvg, game.CpuTempPeak, game.ThrottledSecs));
        Assert.Null(records[1].CpuTempAvg);
        Assert.Empty(records[1].Programs);
    }

    // ---- 核心与缓存面板 ----

    [Fact]
    public void Domains_sharing_an_l3_are_grouped_and_described()
    {
        var view = CoresText.Build(Cores, Hybrid);

        var group = Assert.Single(view.Groups);
        Assert.Equal("三级缓存 24 MB · 2 组共用", group.Title);
        var big = group.Domains[0];
        Assert.Equal(("大核", "6 核 12 线程", "每个核心独享 1.25 MB 二级缓存", "63%"), (big.Title, big.Shape, big.Cache, big.Load));
        // 两个逻辑处理器属于同一个核心。
        Assert.Equal(2, Assert.Single(big.Cores).Threads.Count);
        Assert.Equal(("CPU 1", "45%"), (big.Cores[0].Threads[1].Label, big.Cores[0].Threads[1].Load));
        var game = Assert.Single(big.Programs);
        Assert.Equal(("游戏", TagTone.Info, "2.6 核", "跨 2 组"), (game.Role, game.Tone, game.Load, game.Span));
        var small = group.Domains[1];
        Assert.Equal(("小核", "4 个核心共用 2 MB 二级缓存", "—"), (small.Title, small.Cache, small.Load));
        Assert.Equal(["游戏", "让路", "挪开"], small.Programs.Select(p => p.Role));
        Assert.False(small.Cores[0].Threads[0].Measured);
        Assert.Equal(("Microsoft Edge", "降低优先级"), (Assert.Single(view.Unrestricted).Name, view.Unrestricted[0].Role));
        Assert.Equal("6 个大核、4 个小核，16 个线程，分为 2 组", view.Summary);
        Assert.Null(view.Note);
    }

    [Fact]
    public void Processors_without_big_and_small_cores_are_named_by_group()
    {
        var map = Cores;
        var ccds = map with
        {
            Domains = [map.Domains[0] with { L3Group = 0 }, map.Domains[0] with { L3Group = 1, L3Bytes = 32u << 20 }],
        };

        var view = CoresText.Build(ccds, TwoCcds);

        // 两块三级缓存各自一组。
        Assert.Equal(["三级缓存 24 MB", "三级缓存 32 MB"], view.Groups.Select(g => g.Title));
        Assert.Equal(["主核心组", "核心组 2"], view.Groups.Select(g => g.Domains[0].Title));
        Assert.Equal("12 个核心，24 个线程，分为 2 组", view.Summary);
    }

    [Fact]
    public void A_single_cache_or_smart_scheduling_off_is_explained()
    {
        var map = Cores;
        var single = map with { Partitioned = false, Domains = [map.Domains[0]] };

        Assert.Equal("全部核心", CoresText.Build(single, TwoCcds).Groups[0].Domains[0].Title);
        Assert.Contains("共用同一块缓存", CoresText.Build(single, TwoCcds).Note);
        Assert.Contains("智能调度已关闭", CoresText.Build(map with { SmartSchedule = false }, Hybrid).Note);
    }

    [Fact]
    public void Long_program_lists_are_cut_with_a_count()
    {
        var map = Cores;
        var many = Enumerable.Range(0, CoresText.MaxPrograms + 3)
            .Select(i => new PlacedProgram((uint)i, $"p{i}", "", Verdict.Aside, 0, 1)).ToList();
        var view = CoresText.Build(map with { Domains = [map.Domains[0], map.Domains[1] with { Programs = many }] }, Hybrid);

        var small = view.Groups[0].Domains[1];
        Assert.Equal(CoresText.MaxPrograms, small.Programs.Count);
        Assert.Equal("还有 3 个程序", small.More);
    }

    // ---- 日志 ----

    [Fact]
    public void Every_event_reads_as_a_sentence()
    {
        var page = TestSupport.Fixture<EventPage>("events.json");

        var rows = page.Entries.Select(e => EventText.Row(e, Hybrid, Now)).ToList();

        Assert.All(rows, r => Assert.False(string.IsNullOrWhiteSpace(r.Title)));
        Assert.All(rows, r => Assert.DoesNotContain("{", r.Title));
        Assert.Equal("League of Legends：按游戏调度，使用大核和 1 组小核", rows[1].Title);
        Assert.Equal("负载 2.6 核 · 磁盘 3 MB/s", rows[1].Detail);
        Assert.Equal(@"c:\riot games\league of legends.exe", rows[1].IconPath);
        Assert.Equal(("League of Legends：恢复原样", "负载 0 核 · 在前台"), (rows[2].Title, rows[2].Detail));
        Assert.Equal("切换到G 模式模式", rows.Single(r => r.Title.StartsWith("切换到G")).Title);
        Assert.Equal("最小处理器状态（等级 1）被其他程序改成 100，已改回 5", rows.Single(r => r.Title.StartsWith("最小")).Title);
        Assert.Equal("关闭快速启动、资源管理器默认打开“此电脑”", rows.Single(r => r.Title == "应用了 2 项系统优化").Detail);
        Assert.Equal(TagTone.Notice, rows.Single(r => r.Title.StartsWith("处理器开始被限速")).Tone);
    }

    [Fact]
    public void Core_ranges_follow_the_processor_kind()
    {
        Assert.Equal("大核和 2 组小核", CoreWording.Span(new CoreSpan(false, true, 2), Hybrid));
        Assert.Equal("主核心组", CoreWording.Span(new CoreSpan(false, true, 0), TwoCcds));
        Assert.Equal("其他核心组", CoreWording.Span(new CoreSpan(false, false, 1), TwoCcds));
        Assert.Equal("所有核心", CoreWording.Span(new CoreSpan(true, true, 1), Hybrid));
        Assert.Equal("小核", CoreWording.SpanOfMask(0b110, Hybrid));
        Assert.Null(CoreWording.SpanOfMask(0, Hybrid));
        Assert.Equal(("512 KB", "1.25 MB", "24 MB"), (CoreWording.CacheSize(512 * 1024), CoreWording.CacheSize(1280 * 1024), CoreWording.CacheSize(24u << 20)));
    }

    [Fact]
    public void Old_events_show_their_date()
    {
        var entry = new EventEntry(1, (ulong)Now.AddDays(-3).ToUnixTimeMilliseconds(), EventCategory.System, new ResumedEvent());

        var row = EventText.Row(entry, Hybrid, Now);

        Assert.Contains("月", row.Day);
        Assert.Equal(8, row.Time.Length);
        Assert.Null(EventText.Row(entry with { AtUnixMs = (ulong)Now.ToUnixTimeMilliseconds() }, Hybrid, Now).Day);
        Assert.Equal("开始调度", row.Title);
    }

    // ---- 使用记录 ----

    [Fact]
    public void A_game_record_lists_programs_cores_and_heat()
    {
        var game = TestSupport.Fixture<List<SessionRecord>>("records.json")[0];

        var view = RecordText.Build(game, Hybrid, Now);

        Assert.Equal("League of Legends · Riot Client", view.Title);
        Assert.Equal("40 分钟", view.Duration);
        Assert.Equal("G 模式、全力", view.Modes);
        Assert.True(view.Game && view.Throttled);
        Assert.Equal(
            [
                "League of Legends 使用大核和 1 组小核，最高负载 3.1 核",
                "移到小核 23 个程序，让路 4 个后台程序",
                "处理器占用平均 42%，最高 88%",
                "处理器温度平均 87°，最高 99°",
                "被限速 3 分钟",
            ],
            view.Facts);
    }

    [Fact]
    public void A_record_without_temperatures_or_programs_still_reads_well()
    {
        var work = TestSupport.Fixture<List<SessionRecord>>("records.json")[1];

        var view = RecordText.Build(work, TwoCcds, Now);

        Assert.Equal("重点程序", view.Title);
        Assert.Null(view.IconPath);
        Assert.DoesNotContain(view.Facts, f => f.Contains("温度"));
        Assert.Equal("没有被限速", view.Facts[^1]);
    }
}
