use crate::core::types::LyricsLine;

/// Parse text in standard LRC format into sorted vector of LyricsLine
pub fn parse_lrc(text: &str) -> Vec<LyricsLine> {
    let mut lines = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check if line contains timestamp e.g. [01:23.45]
        let mut remaining = trimmed;
        let mut timestamps = Vec::new();

        while remaining.starts_with('[') {
            if let Some(close_bracket) = remaining.find(']') {
                let tag = &remaining[1..close_bracket];
                if let Some(ms) = parse_timestamp(tag) {
                    timestamps.push(ms);
                }
                remaining = remaining[close_bracket + 1..].trim_start();
            } else {
                break;
            }
        }

        let lyric_text = remaining.trim().to_string();
        for ts in timestamps {
            lines.push(LyricsLine {
                timestamp_ms: ts,
                text: lyric_text.clone(),
            });
        }
    }

    lines.sort_by_key(|l| l.timestamp_ms);
    lines
}

/// Parse [mm:ss.xx] or [mm:ss.xxx] into milliseconds
fn parse_timestamp(tag: &str) -> Option<u64> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let minutes: u64 = parts[0].parse().ok()?;
    let sec_parts: Vec<&str> = parts[1].split('.').collect();
    if sec_parts.is_empty() {
        return None;
    }

    let seconds: u64 = sec_parts[0].parse().ok()?;
    let mut ms: u64 = 0;

    if sec_parts.len() > 1 {
        let frac_str = sec_parts[1];
        if frac_str.len() == 2 {
            // Centiseconds (e.g. 50 -> 500ms)
            let centi: u64 = frac_str.parse().ok()?;
            ms = centi * 10;
        } else if frac_str.len() >= 3 {
            // Milliseconds (e.g. 500 -> 500ms)
            let raw_ms: u64 = frac_str[..3].parse().ok()?;
            ms = raw_ms;
        } else if frac_str.len() == 1 {
            let tenths: u64 = frac_str.parse().ok()?;
            ms = tenths * 100;
        }
    }

    Some(minutes * 60_000 + seconds * 1000 + ms)
}

/// Find index of the currently active lyric line for a playback timestamp
pub fn find_active_line(lines: &[LyricsLine], position_ms: u64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }

    let mut active = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.timestamp_ms <= position_ms {
            active = Some(idx);
        } else {
            break;
        }
    }

    active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_lrc() {
        let sample = r#"
[ti:Sample Song]
[ar:Artist]
[00:05.50]First line of song
[00:12.80]Second line of song
[00:25.123]Third line of song with 3-digit ms
"#;

        let parsed = parse_lrc(sample);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].timestamp_ms, 5_500);
        assert_eq!(parsed[0].text, "First line of song");
        assert_eq!(parsed[1].timestamp_ms, 12_800);
        assert_eq!(parsed[2].timestamp_ms, 25_123);
    }

    #[test]
    fn test_find_active_line() {
        let lines = vec![
            LyricsLine {
                timestamp_ms: 10_000,
                text: "Line 1".to_string(),
            },
            LyricsLine {
                timestamp_ms: 20_000,
                text: "Line 2".to_string(),
            },
            LyricsLine {
                timestamp_ms: 30_000,
                text: "Line 3".to_string(),
            },
        ];

        // Before first line
        assert_eq!(find_active_line(&lines, 5_000), None);

        // Exactly at first line
        assert_eq!(find_active_line(&lines, 10_000), Some(0));

        // Between first and second line
        assert_eq!(find_active_line(&lines, 15_000), Some(0));

        // At second line
        assert_eq!(find_active_line(&lines, 20_000), Some(1));

        // Past last line
        assert_eq!(find_active_line(&lines, 45_000), Some(2));
    }
}
