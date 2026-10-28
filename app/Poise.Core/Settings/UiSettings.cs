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
            return File.Exists(FilePath)
                ? JsonSerializer.Deserialize<UiSettings>(File.ReadAllText(FilePath)) ?? new UiSettings()
                : new UiSettings();
        }
        catch (Exception e) when (e is IOException or JsonException or UnauthorizedAccessException)
        {
            return new UiSettings();
        }
    }

    public void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath)!);
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // 保存失败不影响使用，下次启动回到默认设置。
        }
    }
}
