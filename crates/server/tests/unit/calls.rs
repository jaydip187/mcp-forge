use super::*;
use chrono::TimeZone;

fn stat(ts: DateTime<Utc>, tool: &str, status: &str, ms: i64) -> CallStat {
    CallStat {
        ts,
        tool_name: tool.into(),
        status: status.into(),
        duration_ms: ms,
    }
}

#[test]
fn percentiles_use_nearest_rank() {
    assert_eq!(percentile(&[], 0.5), None);
    assert_eq!(percentile(&[7], 0.95), Some(7));
    let v: Vec<u64> = (1..=100).collect();
    assert_eq!(percentile(&v, 0.5), Some(50));
    assert_eq!(percentile(&v, 0.95), Some(95));
}

#[test]
fn window_starts_on_bucket_boundaries() {
    let now = Utc.with_ymd_and_hms(2026, 10, 30, 14, 37, 5).unwrap();
    assert_eq!(
        Window::Day.start(now),
        Utc.with_ymd_and_hms(2026, 10, 29, 15, 0, 0).unwrap()
    );
    assert_eq!(
        Window::Week.start(now),
        Utc.with_ymd_and_hms(2026, 10, 24, 0, 0, 0).unwrap()
    );
    assert_eq!(Window::parse("30d"), Some(Window::Month));
    assert_eq!(Window::parse("1y"), None);
}

#[test]
fn summary_groups_by_tool_and_bucket() {
    let now = Utc.with_ymd_and_hms(2026, 10, 30, 14, 37, 0).unwrap();
    let h = |hours_ago: i64| now - Duration::hours(hours_ago);
    let stats = [
        stat(h(30), "old", "ok", 1), // before the window
        stat(h(5), "get_repo", "ok", 100),
        stat(h(5), "get_repo", "error", 300),
        stat(h(1), "get_repo", "ok", 200),
        stat(h(0), "nope", "unknown_tool", 0),
    ];
    let s = summarize(Window::Day, now, &stats);
    assert_eq!((s.total, s.errors), (4, 2));
    assert_eq!((s.window, s.bucket), ("24h", "hour"));
    assert_eq!(s.p50_ms, Some(100));
    assert_eq!(s.p95_ms, Some(300));
    assert_eq!(s.buckets.len(), 24);
    assert_eq!(
        s.buckets[23].start,
        Utc.with_ymd_and_hms(2026, 10, 30, 14, 0, 0).unwrap()
    );
    let counts = |i: usize| (s.buckets[i].ok, s.buckets[i].errors);
    assert_eq!(counts(23), (0, 1));
    assert_eq!(counts(22), (1, 0));
    assert_eq!(counts(18), (1, 1));
    assert_eq!(s.buckets.iter().map(|b| b.ok + b.errors).sum::<u64>(), 4);

    let repo = &s.tools[0];
    assert_eq!(repo.tool_name, "get_repo");
    assert_eq!((repo.calls, repo.errors), (3, 1));
    assert_eq!((repo.p50_ms, repo.p95_ms), (Some(200), Some(300)));
    assert_eq!(repo.last_call, h(1));
    assert_eq!(s.tools[1].tool_name, "nope");
    assert_eq!(s.tools.len(), 2);
}

#[test]
fn empty_summary() {
    let s = summarize(Window::Month, Utc::now(), &[]);
    assert_eq!((s.total, s.errors, s.p50_ms), (0, 0, None));
    assert_eq!(s.buckets.len(), 30);
    assert!(s.tools.is_empty());
}
