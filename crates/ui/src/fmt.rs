//! Human-readable formatting for numbers, sizes and dates.

pub fn playtime(minutes: u32) -> String {
    match minutes {
        0 => "Never played".into(),
        m if m < 60 => format!("{m} minutes"),
        m => format!("{:.1} hours", m as f64 / 60.0),
    }
}

pub fn bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

/// "12.3 MB/s"; "-" when the speed is not known yet.
pub fn speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec < 1.0 { "-".into() } else { format!("{}/s", bytes(bytes_per_sec as u64)) }
}

/// "1 h 05 min", "4 min 10 s", "12 s".
pub fn duration(secs: u64) -> String {
    match secs {
        s if s >= 3600 => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
        s if s >= 60 => format!("{} min {:02} s", s / 60, s % 60),
        s => format!("{s} s"),
    }
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(unix: u64, now: u64) -> String {
    match now.saturating_sub(unix) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

/// "Today", "Yesterday", or "12 Nov" / "12 Nov 2024" for older years.
pub fn last_played(unix: u64, now: u64) -> String {
    if unix == 0 {
        return "Never".into();
    }
    let (day, today) = (unix / 86_400, now / 86_400);
    match today.saturating_sub(day) {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        _ => {}
    }
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d) = civil(day as i64);
    let (ny, _, _) = civil(today as i64);
    if y == ny { format!("{d} {}", MONTHS[m - 1]) } else { format!("{d} {} {y}", MONTHS[m - 1]) }
}

/// Days since 1970-01-01 → (year, month 1-12, day). Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, usize, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as usize;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "14:05" for a Unix time, shifted by `offset` seconds east of UTC.
pub fn clock(unix: u64, offset: i64) -> String {
    let t = (unix as i64 + offset).rem_euclid(86_400);
    format!("{:02}:{:02}", t / 3600, t % 3600 / 60)
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_wraps_days() {
        assert_eq!(clock(3600 * 14 + 300, 0), "14:05");
        assert_eq!(clock(3600 * 23, 7200), "01:00");
        assert_eq!(clock(60, -3600), "23:01");
    }

    #[test]
    fn dates() {
        let now = 1_795_200_000; // 2026-11-20
        assert_eq!(last_played(now - 10, now), "Today");
        assert_eq!(last_played(now - 86_400, now), "Yesterday");
        assert_eq!(last_played(1_700_000_000, now), "14 Nov 2023");
        assert_eq!(civil(0), (1970, 1, 1));
    }

    #[test]
    fn rates_and_durations() {
        assert_eq!(speed(0.0), "-");
        assert_eq!(speed(2.0 * 1024.0 * 1024.0), "2.0 MB/s");
        assert_eq!(duration(12), "12 s");
        assert_eq!(duration(250), "4 min 10 s");
        assert_eq!(duration(3900), "1 h 05 min");
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 7200), "2 h ago");
    }

    #[test]
    fn sizes_and_time() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(playtime(90), "1.5 hours");
        assert_eq!(playtime(0), "Never played");
    }
}
