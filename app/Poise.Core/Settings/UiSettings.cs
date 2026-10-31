using System.Text.Json;

namespace Poise.Core.Settings;

public enum ThemeChoice { System, Light, Dark }

/// <summary>程序偏好页"正在运行""系统"两栏按哪一列排序（方向另存）。</summary>
public enum AppSort { Name, Cpu, Memory }

/// <summary>
/// 只属于界面的本地设置（语言、外观、托盘图标提醒、程序列表的排序），保存在用户目录。
/// 托盘代理也读取这个文件（语言、外观、是否显示提醒圆点），字段名改动时两边一起改。
/// </summary>
public sealed class UiSettings
{
    private static readonly string FilePath = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Poise", "ui.json");

    /// <summary>null 表示跟随系统。</summary>
    public string? Language { get; set; }

    public ThemeChoice Theme { get; set; } = ThemeChoice.System;

    /// <summary>需要留意时（温度过高、内存占用偏高等）在托盘图标上显示圆点。</summary>
    public bool TrayAlert { get; set; } = true;

    /// <summary>程序偏好页上次选的排序：哪一列，以及是否倒序。</summary>
    public AppSort AppsSort { get; set; } = AppSort.Name;

    public bool AppsSortDescending { get; set; }

    public static UiSettings Load()
    {
        try
        {
            var settings = File.Exists(FilePath)
                ? JsonSerializer.Deserialize(File.ReadAllText(FilePath), LocalJson.Default.UiSettings) ?? new UiSettings()
                : new UiSettings();
            return settings.Valid();
        }
        catch (Exception e) when (e is IOException or JsonException or UnauthorizedAccessException)
        {
            return new UiSettings();
        }
    }

    /// <summary>枚举按数字保存：降级、手改后读回的取值可能不在范围里（设置页的下拉框会越界），换成默认值。</summary>
    private UiSettings Valid()
    {
        if (!Enum.IsDefined(Theme))
        {
            Theme = ThemeChoice.System;
        }
        if (!Enum.IsDefined(AppsSort))
        {
            AppsSort = AppSort.Name;
        }
        return this;
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath)!);
            // 先写临时文件再替换：托盘代理收到通知去读时，不会读到写了一半的文件。
            var temp = FilePath + ".tmp";
            using (var file = new FileStream(temp, FileMode.Create, FileAccess.Write))
            {
                JsonSerializer.Serialize(file, this, LocalJson.Default.UiSettings);
                file.Flush(flushToDisk: true);
            }
            File.Move(temp, FilePath, overwrite: true);
            Lifecycle.Agent.PrefsChanged();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 保存失败不影响使用，下次启动回到默认设置。
        }
    }
}
