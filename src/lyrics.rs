//! LRC parsing for the synced-lyrics popup.
//!
//! The FLAC `LYRICS` tag carries line timestamps (`[mm:ss.xxx]`, sometimes
//! several per line for repeated choruses). This module extracts them into
//! sorted `(time, text)` pairs, stripping word timestamps `<…>` and voice
//! labels (`v1:` etc.) — the same cleanup the plain view already does.
use crate::app::App;

/// One timed lyric line.
#[derive(Debug, Clone, PartialEq)]
pub struct LrcLine {
    pub time: f64,
    pub text: String,
}

/// All timed lines of a track, sorted by time.
#[derive(Debug, Clone)]
pub struct LrcLines {
    pub lines: Vec<LrcLine>,
}

/// Parse `mm:ss` / `mm:ss.xx` / `mm:ss.xxx` inside a `[...]` group.
fn parse_time(group: &str) -> Option<f64> {
    let (mm, ss) = group.split_once(':')?;
    let mins: f64 = mm.trim().parse().ok()?;
    let secs: f64 = ss.trim().parse().ok()?;
    Some(mins * 60.0 + secs)
}

/// Parse an LRC tag into timed lines. `None` when no line carries a
/// timestamp (plain lyrics — the popup falls back to scrollable text).
pub fn parse_lrc(raw: &str) -> Option<LrcLines> {
    let mut lines: Vec<LrcLine> = Vec::new();
    for raw_line in raw.lines() {
        let line = raw_line.trim_end_matches('\r');
        let mut times: Vec<f64> = Vec::new();
        let mut rest = line;
        // consume every leading [...] group
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            let group = &rest[1..close];
            match parse_time(group) {
                Some(t) => times.push(t),
                None => {
                    // first group is a metadata header ([ti:...]) → skip line
                    // ONLY when we haven't seen timestamps yet; otherwise it
                    // is trailing junk and the text after it matters
                    if times.is_empty() {
                        rest = "";
                        break;
                    }
                    break;
                }
            }
            rest = &rest[close + 1..];
        }
        if times.is_empty() {
            continue;
        }
        let text = App::clean_lyrics(rest).trim().to_string();
        for t in times {
            lines.push(LrcLine { time: t, text: text.clone() });
        }
    }
    if lines.is_empty() {
        return None;
    }
    lines.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
    Some(LrcLines { lines })
}

/// Index of the line currently singing: the last one whose time has passed
/// (with a small lead so the highlight lands as the line starts).
pub fn active_line(l: &LrcLines, time_pos: f64) -> Option<usize> {
    let lead = 0.3;
    let mut idx = None;
    for (i, line) in l.lines.iter().enumerate() {
        if line.time <= time_pos + lead {
            idx = Some(i);
        } else {
            break;
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lrc_extracts_lines_and_timestamps() {
        // the EXACT LRC sample from the chief's screenshot
        let raw = concat!(
            "[00:18.465]v1:<00:18.465>Through <00:19.102><00:19.102>drought <00:19.488><00:19.488>and famine\n",
            "[00:21.818]v1:<00:21.818>Natural <00:22.784><00:22.784>disasters\n",
        );
        let l = parse_lrc(raw).unwrap();
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].time, 18.465);
        assert_eq!(l.lines[0].text, "Through drought and famine");
        assert_eq!(l.lines[1].time, 21.818);
        assert_eq!(l.lines[1].text, "Natural disasters");
    }

    #[test]
    fn parse_lrc_multi_timestamp_repeats() {
        let l = parse_lrc("[00:12.00][01:34.00]chorus\n").unwrap();
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].time, 12.0);
        assert_eq!(l.lines[1].time, 94.0);
        assert_eq!(l.lines[0].text, "chorus");
        assert_eq!(l.lines[1].text, "chorus");
    }

    #[test]
    fn parse_lrc_none_without_timestamps() {
        assert!(parse_lrc("plain lyrics\nsecond line\n").is_none());
    }

    #[test]
    fn parse_lrc_drops_metadata_headers() {
        let l = parse_lrc("[ti:Some Song]\n[by:someone]\n[00:01.00]hi\n").unwrap();
        assert_eq!(l.lines.len(), 1);
        assert_eq!(l.lines[0].text, "hi");
    }

    #[test]
    fn parse_lrc_sorts_by_time() {
        let l = parse_lrc("[00:30.00]later\n[00:10.00]earlier\n[00:20.00]middle\n").unwrap();
        let times: Vec<f64> = l.lines.iter().map(|x| x.time).collect();
        assert_eq!(times, vec![10.0, 20.0, 30.0]);
        assert_eq!(l.lines[0].text, "earlier");
    }

    #[test]
    fn parse_lrc_timestamp_formats() {
        let l = parse_lrc("[00:05]a\n[00:05.25]b\n[01:05.255]c\n").unwrap();
        assert_eq!(l.lines[0].time, 5.0);
        assert_eq!(l.lines[1].time, 5.25);
        assert_eq!(l.lines[2].time, 65.255);
    }

    #[test]
    fn parse_lrc_strips_crlf() {
        let l = parse_lrc("[00:01.00]windows line\r\n[00:02.00]second\r\n").unwrap();
        assert_eq!(l.lines[0].text, "windows line");
        assert_eq!(l.lines[1].text, "second");
    }

    #[test]
    fn active_line_picks_last_elapsed() {
        let l = parse_lrc("[00:10.00]a\n[00:20.00]b\n[00:30.00]c\n").unwrap();
        assert_eq!(active_line(&l, 25.0), Some(1));
        assert_eq!(active_line(&l, 5.0), None);
        assert_eq!(active_line(&l, 100.0), Some(2));
        assert_eq!(active_line(&l, 10.2), Some(0), "small lead lands the line");
    }
}
