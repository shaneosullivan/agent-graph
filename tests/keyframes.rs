//! Keyframes: the reducer's whole state at a point in a log, so the log can
//! carry on from there without what came before (the site keeps only a
//! log's last two keyframes' worth).

mod common;

use std::time::Duration;

use agent_graph::adapter::Capture;
use agent_graph::event::Envelope;
use agent_graph::keyframe;
use agent_graph::reducer::{self, KEYFRAME_PART, Options, is_keyframe, sort_key};
use agent_graph::timeline::{self, Timed};
use common::*;
use serde_json::{Value, json};

fn ev(ms: u64, node: &str, kind: &str, data: Value) -> Envelope {
    let at = t0() + Duration::from_millis(ms);
    Envelope {
        v: 1,
        id: ulid::Ulid::from_datetime(at).to_string(),
        ts: humantime::format_rfc3339_millis(at).to_string(),
        kind: kind.into(),
        node: node.into(),
        parent: None,
        source: None,
        trace: None,
        data,
    }
}

fn with_parent(mut e: Envelope, parent: &str) -> Envelope {
    e.parent = Some(parent.into());
    e
}

const CHILD: &str = "claude-code:c0ffee00-0000-4000-8000-000000000001";
const OTHER: &str = "claude-code:c0ffee00-0000-4000-8000-000000000002";
const QUICK: &str = "claude-code:c0ffee00-0000-4000-8000-000000000003";
const UNDER: &str = "claude-code:c0ffee00-0000-4000-8000-000000000004";
const GUESSED: &str = "claude-code:c0ffee00-0000-4000-8000-000000000005";
const ENDED: &str = "claude-code:c0ffee00-0000-4000-8000-000000000006";
const EARLY: &str = "claude-code:c0ffee00-0000-4000-8000-000000000007";

/// A log with a bit of everything, so that everything the reducer keeps
/// matters across some point in it: the session fixture (tasks, agents,
/// requests, waits, messages, a question); a session it starts from its
/// shell, which ends, with a late status, and is resumed; one that ends as
/// soon as it starts; one linked by its process; one paired by a guess
/// that the request's return puts right; and one whose start sorts before
/// its request.
fn log() -> Vec<Envelope> {
    let mut events = translate(
        &fixture("claude-code/session.jsonl"),
        Capture { bodies: true },
    );
    events.extend([
        ev(30_000, SESSION, "spawn.requested", json!({"call_id": "b1", "kind": "session", "agent_type": "claude", "purpose": "Review it"})),
        ev(30_000, SESSION, "wait.started", json!({"wait_id": "b1", "on": CHILD, "reason": "Review it"})),
        with_parent(ev(30_500, CHILD, "session.started", json!({"cwd": "/w", "link_method": "env"})), SESSION),
        ev(31_000, CHILD, "status", json!({"state": "working"})),
        ev(32_000, CHILD, "tasks.updated", json!({"items": [{"id": "1", "text": "Read", "status": "in_progress"}]})),
        ev(33_000, CHILD, "message.sent", json!({"message_id": "m1", "to": SESSION, "summary": "Done"})),
        // Late, just after its end (`ended`).
        ev(40_000, CHILD, "session.ended", json!({})),
        ev(41_000, CHILD, "status", json!({"state": "idle"})),
        ev(40_500, SESSION, "spawn.returned", json!({"call_id": "b1", "child": CHILD})),
        ev(40_500, SESSION, "wait.ended", json!({"wait_id": "b1"})),
        ev(41_000, OTHER, "session.started", json!({"process": "77@1", "ancestors": ["9@1"]})),
        ev(41_500, OTHER, "status", json!({"state": "input_required", "summary": "May I?"})),
        ev(50_000, CHILD, "session.started", json!({"source": "resume"})),
        ev(50_100, CHILD, "status", json!({"state": "working"})),
        // Ends as soon as it starts (`started`): only an idle is late.
        ev(62_000, QUICK, "session.started", json!({})),
        ev(63_000, QUICK, "session.ended", json!({})),
        ev(63_500, QUICK, "status", json!({"state": "idle"})),
        ev(63_600, QUICK, "status", json!({"state": "input_required", "summary": "And?"})),
        // Linked by its process (`processes`).
        ev(70_000, UNDER, "session.started", json!({"process": "78@1", "ancestors": ["77@1"]})),
        // Paired by a guess, put right (`requesters`), and waited on until
        // it ends (the waits on each node).
        ev(80_000, SESSION, "spawn.requested", json!({"call_id": "r1", "kind": "session", "agent_type": "claude"})),
        ev(80_100, SESSION, "spawn.requested", json!({"call_id": "r2", "kind": "session", "agent_type": "claude"})),
        with_parent(ev(80_500, GUESSED, "session.started", json!({"link_method": "env"})), SESSION),
        ev(80_600, SESSION, "wait.started", json!({"wait_id": "w3", "on": GUESSED})),
        ev(81_000, SESSION, "spawn.returned", json!({"call_id": "r2", "child": GUESSED})),
        ev(85_000, GUESSED, "session.ended", json!({})),
        // Ended for good, with a late Stop (`ended`).
        ev(90_000, ENDED, "session.started", json!({})),
        ev(90_100, ENDED, "status", json!({"state": "working"})),
        ev(95_000, ENDED, "session.ended", json!({})),
        ev(95_500, ENDED, "status", json!({"state": "idle"})),
    ]);
    // Paired with a request that sorts after its start, in the same
    // millisecond (`unpaired_runs`), once the calls before have returned.
    events.push(ev(99_000, SESSION, "status", json!({"state": "idle"})));
    let mut early = [
        with_parent(
            ev(
                100_000,
                EARLY,
                "session.started",
                json!({"link_method": "env"}),
            ),
            SESSION,
        ),
        ev(
            100_000,
            SESSION,
            "spawn.requested",
            json!({"call_id": "e1", "kind": "session", "agent_type": "claude"}),
        ),
    ];
    for (i, e) in early.iter_mut().enumerate() {
        e.id = format!("{}{i:016}", &e.id[..10]);
    }
    events.extend(early);
    events.sort_by_cached_key(sort_key);
    events
}

fn opts() -> Options {
    Options {
        now: t0() + Duration::from_secs(120),
        stale_after: Duration::from_secs(30 * 60),
    }
}

/// The graph as JSON (everything the page is sent).
fn shown(g: reducer::Graph) -> Value {
    serde_json::to_value(g).unwrap()
}

/// From every point in the log, a keyframe and the events after it make
/// the same graph as the whole log.
#[test]
fn a_keyframe_carries_on_exactly() {
    let events = log();
    let whole = shown(reducer::reduce(events.clone(), &opts()));
    for cut in 1..=events.len() {
        let parts = reducer::keyframe(None, &events[..cut], KEYFRAME_PART).unwrap();
        assert_eq!(parts.len(), 1, "a small state is one line");
        let base = reducer::merge_keyframe(&parts).unwrap();
        let from = reducer::reduce_from(Some(&base), events[cut..].to_vec(), &opts());
        assert_eq!(shown(from), whole, "cut after {cut} events");
    }
    assert!(
        reducer::keyframe(None, &[], KEYFRAME_PART).is_none(),
        "nothing before it"
    );
}

/// A state of any size is written, split into as many lines as it takes,
/// each within the limit, which merge back into the same state, in any
/// order; a keyframe can carry on from another.
#[test]
fn a_big_keyframe_is_split_and_merged() {
    let events = log();
    let whole = shown(reducer::reduce(events.clone(), &opts()));
    let cut = events.len() / 2;
    let parts = reducer::keyframe(None, &events[..cut], 100).unwrap();
    assert!(parts.len() > 10, "{} parts", parts.len());
    for part in &parts {
        // As it's written: escaped, in quotes.
        assert!(serde_json::to_string(&part.data["text"]).unwrap().len() <= 102);
        assert_eq!(part.node, reducer::KEYFRAME_NODE);
        assert_eq!(part.ts, parts[0].ts);
    }
    // Right after the last event it stands for, in order.
    assert!(parts[0].id > events[cut - 1].id && parts[0].id < events[cut].id);
    assert!(parts.windows(2).all(|w| w[0].id < w[1].id));
    let mut shuffled = parts.clone();
    shuffled.reverse();
    let base = reducer::merge_keyframe(&shuffled).unwrap();
    let from = reducer::reduce_from(Some(&base), events[cut..].to_vec(), &opts());
    assert_eq!(shown(from), whole);
    // Missing a part, or with one twice, it's no keyframe.
    assert!(reducer::merge_keyframe(&parts[1..]).is_none());
    let without = |i: usize| -> Vec<Envelope> {
        parts
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, p)| p.clone())
            .collect()
    };
    assert!(reducer::merge_keyframe(&without(1)).is_none());
    let mut twice = without(1);
    twice.push(parts[0].clone());
    assert!(reducer::merge_keyframe(&twice).is_none());
    let mut extra = parts.clone();
    extra.push(parts[1].clone());
    assert!(
        reducer::merge_keyframe(&extra).is_none(),
        "one more than it has"
    );
    // Parts that don't agree on how many there are.
    let mut disagree = parts.clone();
    disagree[1].data["parts"] = json!(parts.len() + 1);
    assert!(reducer::merge_keyframe(&disagree).is_none());
    // One from another.
    let later = events.len() - 5;
    let next = reducer::keyframe(Some(&base), &events[cut..later], KEYFRAME_PART).unwrap();
    let base = reducer::merge_keyframe(&next).unwrap();
    let from = reducer::reduce_from(Some(&base), events[later..].to_vec(), &opts());
    assert_eq!(shown(from), whole);
}

/// A keyframe written before R41, R53 and R54 changed what the reducer
/// keeps (it has `waiting_on`, now made from the nodes, and neither
/// `unpaired_runs` nor who made each child's request) still loads: a log
/// shared and trimmed then starts with one. Carrying on from it (after the
/// log's first 8 events, as d0c927d's code wrote it) makes the graph the
/// whole log does, but that the children paired before it don't say which
/// node asked for them.
#[test]
fn a_keyframe_from_before_still_loads() {
    let envelopes = |path: &str| -> Vec<Envelope> {
        fixture(path)
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect()
    };
    let mut events = envelopes("keyframes/d0c927d-log.jsonl");
    events.sort_by_cached_key(sort_key);
    let parts = envelopes("keyframes/d0c927d-keyframe.jsonl");
    let base = reducer::merge_keyframe(&parts).expect("the keyframe loads");
    assert_eq!(base.id, format!("{}~000000", events[7].id));
    let without_requesters = |g: reducer::Graph| {
        let mut json = shown(g);
        for node in json["nodes"].as_object_mut().unwrap().values_mut() {
            node.as_object_mut().unwrap().remove("requested_by");
        }
        json
    };
    let from = reducer::reduce_from(Some(&base), events[8..].to_vec(), &opts());
    let whole = reducer::reduce(events, &opts());
    assert_eq!(without_requesters(from), without_requesters(whole));
}

/// A keyframe anywhere but a log's start stands for events already there:
/// it changes nothing.
#[test]
fn a_keyframe_after_the_start_changes_nothing() {
    let events = log();
    let whole = shown(reducer::reduce(events.clone(), &opts()));
    let cut = events.len() / 2;
    let mut with = events.clone();
    with.splice(
        cut..cut,
        reducer::keyframe(None, &events[..cut], 3000).unwrap(),
    );
    assert_eq!(shown(reducer::reduce(with, &opts())), whole);
}

/// A keyframe's ids are never an event's: events from one hook have ids one
/// after another, and the one after the last it stands for is still read.
#[test]
fn a_keyframes_ids_are_its_own() {
    let events = log();
    // Two from one call: consecutive ids.
    let pair: Vec<Envelope> = events
        .windows(2)
        .find(|w| w[0].ts == w[1].ts)
        .map(|w| w.to_vec())
        .unwrap();
    let before: Vec<Envelope> = events
        .iter()
        .take_while(|e| e.id != pair[1].id)
        .cloned()
        .collect();
    let parts = reducer::keyframe(None, &before, KEYFRAME_PART).unwrap();
    assert!(
        parts
            .iter()
            .all(|p| p.id != pair[1].id && p.id > pair[0].id && p.id < pair[1].id)
    );
    // As the site reads it, the event after it is there.
    let mut text = String::new();
    for e in parts.iter().chain(&events[before.len()..]) {
        text += &(serde_json::to_string(e).unwrap() + "\n");
    }
    let sent = keyframe::trim(text.as_bytes(), 1_000_000);
    assert_eq!(sent.of, events.len() - before.len());
}

/// A keyframe from a pasted log can say anything: one the reducer couldn't
/// have made is refused, and never makes it fail.
#[test]
fn a_keyframe_that_couldnt_be_is_refused() {
    let events = log();
    let base =
        reducer::merge_keyframe(&reducer::keyframe(None, &events, KEYFRAME_PART).unwrap()).unwrap();
    let text = base.data["text"].as_str().unwrap();
    let state: Value = serde_json::from_slice(&reducer::unpack_state(text).unwrap()).unwrap();
    let with = |state: &Value, part: usize, parts: usize| {
        let mut e = base.clone();
        let text = reducer::pack_state(state.to_string().as_bytes());
        e.data = json!({"part": part, "parts": parts, "text": text});
        e
    };
    // A node under another's id.
    let mut moved = state.clone();
    let node = moved["nodes"][SESSION].clone();
    moved["nodes"]["x:elsewhere"] = node;
    // The same request twice.
    let mut twice = state.clone();
    let spawn = twice["nodes"][SESSION]["spawns"][0].clone();
    twice["nodes"][SESSION]["spawns"]
        .as_array_mut()
        .unwrap()
        .push(spawn);
    // Indexes naming nodes there aren't.
    let missing = |index: &str, value: Value| {
        let mut state = state.clone();
        state[index]["somewhere"] = value;
        state
    };
    // A child under a node that isn't its parent.
    let mut adopted = state.clone();
    adopted["nodes"][SESSION]["children"]
        .as_array_mut()
        .unwrap()
        .push(json!(QUICK));
    let mut runs = state.clone();
    runs["unpaired_runs"][2] = json!(["x:nobody"]);
    for bad in [
        with(&moved, 0, 1),
        with(&twice, 0, 1),
        with(&adopted, 0, 1),
        with(&runs, 0, 1),
        with(&missing("processes", json!("x:nobody")), 0, 1),
        with(&missing("requesters", json!(["x:nobody"])), 0, 1),
        with(&json!({"nodes": 5}), 0, 1),
        with(&json!("not a state"), 0, 1),
        with(&state, 0, 2),
        with(&state, 5, 1),
        with(&state, 0, usize::MAX),
    ] {
        assert!(
            reducer::merge_keyframe(std::slice::from_ref(&bad)).is_none(),
            "{}",
            bad.data
        );
        // Nor started from, as the site would.
        let graph = reducer::reduce_from(Some(&bad), events.clone(), &opts());
        assert_eq!(
            shown(graph),
            shown(reducer::reduce(events.clone(), &opts()))
        );
    }
    // A text that isn't packed, or JSON too deep to read, or not JSON.
    let deep = reducer::pack_state("[".repeat(100_000).as_bytes());
    let unfinished = reducer::pack_state(b"{");
    for text in ["{", deep.as_str(), unfinished.as_str()] {
        let mut e = base.clone();
        e.data = json!({"part": 0, "parts": 1, "text": text});
        assert!(reducer::merge_keyframe(&[e]).is_none());
    }
}

/// A small packed text can unpack to a great deal: past a limit, it's
/// refused rather than unpacked.
#[test]
fn a_keyframe_is_unpacked_only_so_far() {
    let text = reducer::pack_state(&[b' '; 100_000]);
    assert!(text.len() < 1000, "{} bytes", text.len());
    assert_eq!(
        reducer::unpack_state_within(&text, 200_000).map(|s| s.len()),
        Some(100_000)
    );
    assert_eq!(reducer::unpack_state_within(&text, 99_999), None);
}

/// A state with many agents packs small: its JSON is very repetitive.
#[test]
fn a_keyframe_packs_its_state_small() {
    let mut events = Vec::new();
    for s in 0..100 {
        let session = format!("claude-code:{s:08}-0000-4000-8000-000000000000");
        events.push(ev(
            s * 1000,
            &session,
            "session.started",
            json!({"cwd": format!("/w/{s}")}),
        ));
        for a in 0..20 {
            let agent = format!("{session}/a{a}");
            let at = s * 1000 + a * 10 + 1;
            events.push(ev(at, &session, "spawn.requested", json!({"call_id": format!("c{a}"), "kind": "agent", "agent_type": "Explore", "purpose": format!("Look into part {a}")})));
            events.push(with_parent(
                ev(
                    at + 1,
                    &agent,
                    "agent.spawned",
                    json!({"agent_type": "Explore"}),
                ),
                &session,
            ));
            events.push(ev(
                at + 2,
                &agent,
                "agent.finished",
                json!({"status": "completed", "summary": format!("Part {a} is fine")}),
            ));
            events.push(ev(
                at + 3,
                &session,
                "spawn.returned",
                json!({"call_id": format!("c{a}"), "child": agent}),
            ));
        }
    }
    events.sort_by_cached_key(sort_key);
    let parts = reducer::keyframe(None, &events, KEYFRAME_PART).unwrap();
    let packed: usize = parts
        .iter()
        .map(|p| p.data["text"].as_str().unwrap().len())
        .sum();
    let base = reducer::merge_keyframe(&parts).unwrap();
    let json = reducer::unpack_state(base.data["text"].as_str().unwrap())
        .unwrap()
        .len();
    let log: usize = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap().len())
        .sum();
    assert!(packed * 8 < json, "{packed} packed, of {json}");
    assert!(packed * 8 < log, "{packed} packed, for a log of {log}");
}

/// A log as the site holds it: the keyframe it starts from (its parts,
/// merged), and its events; a later keyframe stands for what's there.
fn as_site(text: &[u8]) -> Value {
    let lines: Vec<Envelope> = String::from_utf8_lossy(text)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let leading = lines.iter().take_while(|e| is_keyframe(e)).count();
    let base = (leading > 0).then(|| reducer::merge_keyframe(&lines[..leading]).unwrap());
    let events = lines[leading..]
        .iter()
        .filter(|e| !is_keyframe(e))
        .cloned()
        .collect();
    shown(reducer::reduce_from(base.as_ref(), events, &opts()))
}

fn text_of(events: &[Envelope]) -> String {
    events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap() + "\n")
        .collect()
}

fn events_in(text: &[u8]) -> usize {
    String::from_utf8_lossy(text)
        .lines()
        .filter(|l| !l.contains(r#""type":"keyframe""#))
        .count()
}

/// What's sent is the last two keyframes' worth, as the lines were written,
/// and makes the same graph.
#[test]
fn a_log_is_sent_from_its_last_but_one_keyframe() {
    let events = log();
    let every = 10;
    for n in [0, 5, 10, 15, 20, 29, 30, events.len()] {
        let text = text_of(&events[..n]);
        let sent = keyframe::trim(text.as_bytes(), every);
        let keyframes = n / every;
        let from = keyframes.saturating_sub(1) * every;
        assert_eq!((sent.events, sent.of), (n - from, n), "{n} events");
        assert_eq!(events_in(&sent.text), n - from);
        // Where the keyframes stand: at the start (once there are two), and
        // after `every` more events.
        let at: Vec<usize> = sent
            .keyframes
            .iter()
            .map(|&k| events_in(&sent.text[..k]))
            .collect();
        let want: Vec<usize> = match keyframes {
            0 => vec![],
            1 => vec![every],
            _ => vec![0, every],
        };
        assert_eq!(at, want, "{n} events");
        assert_eq!(
            as_site(&sent.text),
            shown(reducer::reduce(events[..n].to_vec(), &opts())),
            "{n} events"
        );
    }
    // Lines in any order are sent in order (a keyframe stands for the first
    // events by time, not by where they were in the text).
    let mut shuffled: Vec<&Envelope> = events.iter().collect();
    shuffled.reverse();
    let text: String = shuffled
        .iter()
        .map(|e| serde_json::to_string(e).unwrap() + "\n")
        .collect();
    let sent = keyframe::trim(text.as_bytes(), every);
    assert_eq!(
        as_site(&sent.text),
        shown(reducer::reduce(events.clone(), &opts()))
    );
    // A keyframe after the start stands for events there already: it's
    // left out, and nothing's started from it.
    let stray = reducer::keyframe(None, &events[..5], KEYFRAME_PART).unwrap();
    let text = text_of(&events[..5]) + &text_of(&stray) + &text_of(&events[5..9]);
    let sent = keyframe::trim(text.as_bytes(), every);
    assert_eq!(String::from_utf8(sent.text).unwrap(), text_of(&events[..9]));
    assert!(sent.keyframes.is_empty());
    // The lines as they were written: a field we don't know, spacing.
    let odd = text_of(&events[..3]).replacen(r#""v":1,"#, r#""v": 1, "extra": [1], "#, 1);
    let sent = keyframe::trim(odd.as_bytes(), every);
    assert_eq!(String::from_utf8(sent.text).unwrap(), odd);
}

/// A log that starts from a keyframe (one pasted from the site, say) is
/// sent the same way.
#[test]
fn a_log_that_starts_from_a_keyframe_is_sent_from_its_own() {
    let events = log();
    let whole = shown(reducer::reduce(events.clone(), &opts()));
    let cut = 7;
    let base = reducer::keyframe(None, &events[..cut], 2000).unwrap();
    let text = text_of(&base) + &text_of(&events[cut..]);
    for every in [5, 1000] {
        let sent = keyframe::trim(text.as_bytes(), every);
        assert!(sent.text.starts_with(br#"{"v":1"#) && sent.keyframes.first() == Some(&0));
        assert_eq!(as_site(&sent.text), whole, "every {every}");
    }
}

/// The timeline of a log that starts from a keyframe starts there, and the
/// graph can be seen at that stop; an event that arrives later but sorts
/// before it is still applied after it.
#[test]
fn the_timeline_starts_at_a_keyframe() {
    let events = log();
    let cut = events.len() / 2;
    let base =
        reducer::merge_keyframe(&reducer::keyframe(None, &events[..cut], KEYFRAME_PART).unwrap())
            .unwrap();
    let mut late = ev(0, SESSION, "status", json!({"state": "working"}));
    late.id = format!("{}Z", &late.id[..25]);
    let mut log: Vec<Timed> = std::iter::once(base.clone())
        .chain(events[cut..].iter().cloned())
        .chain([late])
        .map(Timed::new)
        .collect();
    timeline::sort(&mut log);
    assert_eq!(log[0].event.id, base.id, "it stays first");
    let now = t0() + Duration::from_secs(120);
    let stale = Duration::from_secs(1800);
    let stops: Value =
        serde_json::from_str(&timeline::timeline(&log, SESSION, now, stale).unwrap()).unwrap();
    let first = &stops["stops"][0];
    assert_eq!(first["label"], "Earlier history isn't included");
    assert_eq!(first["id"], base.id.as_str());
    assert_eq!(first["node"], SESSION);
    let (at_base, count, _) = timeline::graph_at(&log, Some(&base.id), now, stale).unwrap();
    assert_eq!(count, 1);
    let at = reducer::event_time(&events[cut - 1]);
    let before = reducer::reduce(
        events[..cut].to_vec(),
        &Options {
            now: at,
            stale_after: stale,
        },
    );
    assert_eq!(shown(at_base).get("nodes"), shown(before).get("nodes"));
}
