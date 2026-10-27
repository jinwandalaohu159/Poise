//! 最小的 INI 编辑：只改目标键，保留文件中其他所有内容和注释。

fn section_name(line: &str) -> Option<&str> {
    let t = line.trim();
    t.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

fn key_of(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.starts_with('#') || t.starts_with(';') {
        return None;
    }
    t.split_once('=').map(|(k, _)| k.trim())
}

/// 返回 (段起始行, 段结束行) 的行号范围；段不存在时返回 None。
fn find_section(lines: &[&str], section: &str) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|l| section_name(l).is_some_and(|s| s.eq_ignore_ascii_case(section)))?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| section_name(l).is_some())
        .map_or(lines.len(), |i| start + 1 + i);
    Some((start, end))
}

pub fn get(content: &str, section: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (start, end) = find_section(&lines, section)?;
    lines[start + 1..end].iter().find_map(|l| {
        let k = key_of(l)?;
        k.eq_ignore_ascii_case(key)
            .then(|| l.split_once('=').map(|(_, v)| v.trim().to_string()))
            .flatten()
    })
}

pub fn set(content: &str, section: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let entry = format!("{key}={value}");
    match find_section(&refs, section) {
        Some((start, end)) => {
            let existing = (start + 1..end)
                .find(|&i| key_of(&lines[i]).is_some_and(|k| k.eq_ignore_ascii_case(key)));
            match existing {
                Some(i) => lines[i] = entry,
                None => lines.insert(end, entry),
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(entry);
        }
    }
    join(lines)
}

pub fn remove(content: &str, section: &str, key: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let Some((start, end)) = find_section(&lines, section) else {
        return content.to_string();
    };
    let kept: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| {
            !(*i > start && *i < end && key_of(l).is_some_and(|k| k.eq_ignore_ascii_case(key)))
        })
        .map(|(_, l)| l.to_string())
        .collect();
    join(kept)
}

fn join(lines: Vec<String>) -> String {
    let mut s = lines.join("\r\n");
    s.push_str("\r\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "# my config\r\n[wsl2]\r\nmemory=8GB\r\n\r\n[experimental]\r\nsparseVhd=true\r\n";

    #[test]
    fn adds_key_to_existing_section_and_keeps_others() {
        let out = set(SAMPLE, "experimental", "autoMemoryReclaim", "gradual");
        assert_eq!(
            get(&out, "experimental", "autoMemoryReclaim").as_deref(),
            Some("gradual")
        );
        assert_eq!(
            get(&out, "experimental", "sparseVhd").as_deref(),
            Some("true")
        );
        assert_eq!(get(&out, "wsl2", "memory").as_deref(), Some("8GB"));
        assert!(out.starts_with("# my config"));
    }

    #[test]
    fn creates_section_when_missing_and_replaces_existing_value() {
        let out = set("", "experimental", "autoMemoryReclaim", "gradual");
        assert_eq!(
            get(&out, "experimental", "autoMemoryReclaim").as_deref(),
            Some("gradual")
        );
        let out = set(&out, "experimental", "autoMemoryReclaim", "dropcache");
        assert_eq!(
            get(&out, "experimental", "autoMemoryReclaim").as_deref(),
            Some("dropcache")
        );
        assert_eq!(out.matches("autoMemoryReclaim").count(), 1);
    }

    #[test]
    fn remove_restores_original_content() {
        let added = set(SAMPLE, "experimental", "autoMemoryReclaim", "gradual");
        let back = remove(&added, "experimental", "autoMemoryReclaim");
        assert_eq!(back, SAMPLE);
    }

    #[test]
    fn keys_in_other_sections_are_untouched() {
        let s = "[a]\r\nk=1\r\n[b]\r\nk=2\r\n";
        assert_eq!(get(&remove(s, "a", "k"), "b", "k").as_deref(), Some("2"));
    }
}
