use super::kind::*;
use super::*;

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

async fn db() -> Db {
    Db::connect("sqlite::memory:").await.unwrap()
}

async fn rec(db: &Db, user: u64, ev: Event, day: &str) {
    db.record_stat(1, user, &ev, d(day)).await.unwrap();
}

fn boards() -> Vec<Board> {
    all_boards(&crate::rules::registry())
}

fn find<'a>(boards: &'a [Board], id: &str) -> &'a Board {
    boards
        .iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("no board {id}"))
}

const DEC: Period = Period::Month(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());

async fn top(db: &Db, id: &str) -> Vec<Entry> {
    db.board(1, DEC, find(&boards(), id), None, Some(10))
        .await
        .unwrap()
}

fn users(es: &[Entry]) -> Vec<i64> {
    es.iter().map(|e| e.user_id).collect()
}

fn judged(s: f32, t: &str) -> Event {
    Event::new(JUDGED).rule("first_message").score(s).text(t)
}

fn punished(secs: f64) -> Event {
    Event::new(PUNISHED)
        .rule("first_message")
        .amount(secs)
        .text("x")
}

#[test]
fn periods() {
    let today = d("2027-01-10");
    let m = |s| Some(Period::Month(d(s)));
    assert_eq!(Period::parse("", today), m("2027-01-01"));
    assert_eq!(Period::parse("last", today), m("2026-12-01"));
    assert_eq!(Period::parse("dec", today), m("2026-12-01"));
    assert_eq!(Period::parse("January", today), m("2027-01-01"));
    assert_eq!(Period::parse("2025-11", today), m("2025-11-01"));
    assert_eq!(Period::parse("all", today), Some(Period::All));
    assert_eq!(Period::parse("ju", today), None);
    assert_eq!(Period::parse("nope", today), None);
    assert_eq!(DEC.pattern(), "2026-12-%");
    assert_eq!(DEC.label(), "December 2026");
}

#[test]
fn streaks_and_improvement() {
    let s = longest_streaks([
        (1, d("2026-12-01")),
        (1, d("2026-12-02")),
        (1, d("2026-12-04")),
        (2, d("2026-12-01")),
        (2, d("2026-12-02")),
        (2, d("2026-12-03")),
        (2, d("2026-12-03")),
    ]);
    assert_eq!((s[0].user_id, s[0].value), (2, 3.0));
    assert_eq!(s[0].day.as_deref(), Some("2026-12-03"));
    assert_eq!((s[1].user_id, s[1].value), (1, 2.0));

    let m = most_improved(
        vec![
            (1, 0.1),
            (1, 0.2),
            (1, 0.8),
            (1, 0.9),
            (2, 0.9),
            (2, 0.9),
            (2, 0.1),
            (2, 0.1),
            (3, 0.1),
            (3, 0.9),
        ],
        4,
    );
    assert_eq!(users(&m), [1], "2 got worse, 3 has too few");
    assert!((m[0].value - 0.7).abs() < 1e-9);
    assert_eq!(m[0].text.as_deref(), Some("0.15 → 0.85"));
}

#[test]
fn text_is_escaped() {
    assert_eq!(quote_text("ho *ho*\n  ho"), "ho \\*ho\\* ho");
    assert!(quote_text(&"a".repeat(500)).ends_with('…'));
    let csv = to_csv(
        &[StatRow {
            created_at: "t".into(),
            day: "2026-12-01".into(),
            user_id: 7,
            rule_id: None,
            kind: "judged".into(),
            score: Some(0.5),
            amount: None,
            text: Some("say \"hi\", ok".into()),
        }],
        |_| "bob".into(),
    );
    assert!(csv.ends_with("7,bob,,judged,0.500,,\"say \"\"hi\"\", ok\"\n"));
}

#[tokio::test]
async fn rule_boards_rank_and_filter() {
    let db = db().await;
    rec(&db, 1, judged(0.9, "merry christmas"), "2026-12-01").await;
    rec(&db, 1, judged(0.8, "ho ho"), "2026-12-02").await;
    rec(&db, 1, judged(0.55, "meh xmas"), "2026-12-03").await;
    rec(&db, 2, judged(0.1, "bah humbug"), "2026-12-01").await;
    rec(&db, 2, judged(0.2, "no"), "2026-12-02").await;
    rec(&db, 2, judged(0.3, "nah"), "2026-12-03").await;
    rec(&db, 2, punished(300.0), "2026-12-01").await;
    rec(&db, 2, punished(60.0), "2026-12-01").await;
    rec(&db, 2, punished(60.0), "2026-12-02").await;
    rec(&db, 1, punished(0.0), "2026-12-03").await;
    // Another month and another guild don't count.
    rec(&db, 3, punished(60.0), "2026-11-30").await;
    db.record_stat(2, 3, &punished(60.0), d("2026-12-01"))
        .await
        .unwrap();

    assert_eq!(users(&top(&db, "naughty").await), [2, 1]);
    // Zero-second "timeouts" (disconnects) don't put you on Hard Time.
    let hard = top(&db, "hard_time").await;
    assert_eq!((users(&hard), hard[0].value), (vec![2], 420.0));
    assert_eq!(top(&db, "longest_sentence").await[0].value, 300.0);
    let worst_day = top(&db, "worst_day").await;
    assert_eq!((worst_day[0].user_id, worst_day[0].value), (2, 2.0));
    assert_eq!(worst_day[0].day.as_deref(), Some("2026-12-01"));
    assert_eq!(top(&db, "repeat_offender").await[0].value, 2.0);
    let naughty_streak = top(&db, "naughty_streak").await;
    assert_eq!(
        (naughty_streak[0].user_id, naughty_streak[0].value),
        (2, 2.0)
    );

    assert_eq!(users(&top(&db, "nice").await), [1, 2]);
    assert_eq!(users(&top(&db, "grinch").await), [2, 1]);
    assert_eq!(
        top(&db, "best").await[0].text.as_deref(),
        Some("merry christmas")
    );
    assert_eq!(
        top(&db, "worst").await[0].text.as_deref(),
        Some("bah humbug")
    );
    assert_eq!(
        top(&db, "close_call").await[0].text.as_deref(),
        Some("meh xmas")
    );
    assert_eq!(
        top(&db, "festive_essay").await[0].text.as_deref(),
        Some("merry christmas")
    );
    let swings = top(&db, "mood_swings").await;
    assert_eq!(users(&swings), [1, 2]);
    assert!((swings[0].value - 0.35).abs() < 1e-6);
    // Nobody passed the "near perfect" bar; user 2 has an 0.1 but not ≤ 0.05.
    assert!(top(&db, "miracle").await.is_empty());
    assert!(top(&db, "ice_cold").await.is_empty());
    // User 1 was punished on the 3rd, so their streak is the 1st-2nd.
    let streak = top(&db, "streak").await;
    assert_eq!((users(&streak), streak[0].value), (vec![1], 2.0));
    assert!(top(&db, "angel").await.is_empty(), "both were punished");
    let first = top(&db, "first_blood").await;
    assert_eq!(first[0].user_id, 2);
    assert_eq!(top(&db, "latest_victim").await[0].user_id, 1);

    assert_eq!(users(&top(&db, "first_message:naughty").await), [2, 1]);
    assert!(top(&db, "vc_greeting:naughty").await.is_empty());
    assert!(db.rule_has_stats(1, DEC, "first_message").await.unwrap());
    assert!(!db.rule_has_stats(1, DEC, "vc_greeting").await.unwrap());

    // Minimum sample size for averages; angels need judgements and no punishments.
    rec(&db, 4, judged(1.0, "perfect"), "2026-12-05").await;
    assert_eq!(users(&top(&db, "nice").await), [1, 2]);
    assert_eq!(users(&top(&db, "miracle").await), [4]);
    rec(&db, 4, judged(0.97, "also perfect"), "2026-12-06").await;
    rec(&db, 4, judged(0.6, "fine"), "2026-12-07").await;
    assert_eq!(users(&top(&db, "angel").await), [4]);
    let reliable = top(&db, "reliable").await;
    assert!(reliable.is_empty(), "nobody has 5 judgements yet");

    let bs = boards();
    let mine = db
        .board(1, DEC, find(&bs, "naughty"), Some(1), None)
        .await
        .unwrap();
    assert_eq!(users(&mine), [1]);
    let all = db
        .board(1, Period::All, find(&bs, "naughty"), None, None)
        .await
        .unwrap();
    assert_eq!(users(&all), [2, 1, 3]);

    let t = db.totals(1, DEC).await.unwrap();
    assert_eq!((t.people, t.judged, t.punishments), (3, 9, 4));
    assert_eq!(t.timeout_secs, 420.0);
    assert_eq!(t.naughtiest_day, Some(("2026-12-01".into(), 2)));
    assert!(t.festive_day.is_none(), "no day has 3 judgements");
    assert!(format_totals(&t).contains("Naughtiest day: **Dec 1**"));

    assert_eq!(db.stat_events(1, DEC).await.unwrap().len(), 13);
    assert_eq!(db.wipe_stats(1, Some(2)).await.unwrap(), 6);
    assert_eq!(users(&top(&db, "naughty").await), [1]);
}

#[tokio::test]
async fn activity_boards() {
    let db = db().await;
    let msg = |words: f64| Event::new(MESSAGE).amount(words);
    for (u, day, words) in [
        (1, "2026-12-01", 3.0),
        (1, "2026-12-01", 40.0),
        (1, "2026-12-01", 2.0),
        (2, "2026-12-01", 5.0),
        (2, "2026-12-02", 5.0),
        (2, "2026-12-03", 5.0),
    ] {
        rec(&db, u, msg(words), day).await;
    }
    rec(&db, 2, Event::new(VC_TIME).amount(3600.0), "2026-12-01").await;
    rec(&db, 2, Event::new(VC_TIME).amount(600.0), "2026-12-01").await;
    rec(&db, 1, Event::new(VC_TIME).amount(1800.0), "2026-12-02").await;
    rec(&db, 1, Event::new(CAPS).text("HELLO THERE"), "2026-12-02").await;
    rec(
        &db,
        1,
        Event::new(CAPS).text("MERRY CHRISTMAS ALL"),
        "2026-12-02",
    )
    .await;

    assert_eq!(users(&top(&db, "chatterbox").await), [1, 2]);
    assert_eq!(top(&db, "wordsmith").await[0].value, 45.0);
    assert_eq!(top(&db, "novelist").await[0].value, 40.0);
    let busiest = top(&db, "busiest_day").await;
    assert_eq!((busiest[0].user_id, busiest[0].value), (1, 3.0));
    assert_eq!(users(&top(&db, "ever_present").await), [2, 1]);
    assert_eq!(users(&top(&db, "yule_log").await), [2, 1]);
    assert_eq!(top(&db, "marathon").await[0].value, 3600.0);
    assert_eq!(top(&db, "vc_day").await[0].value, 4200.0);
    assert_eq!(
        top(&db, "loudest").await[0].text.as_deref(),
        Some("MERRY CHRISTMAS ALL")
    );

    let bs = boards();
    let b = find(&bs, "busiest_day");
    assert_eq!(format_value(b, &busiest[0]), "3 messages (Dec 1)");
    let b = find(&bs, "yule_log");
    assert_eq!(format_value(b, &top(&db, "yule_log").await[0]), "1h10m");
}

#[test]
fn board_ids_are_unique() {
    let boards = boards();
    let mut ids: Vec<_> = boards.iter().map(|b| b.id.as_str()).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}
