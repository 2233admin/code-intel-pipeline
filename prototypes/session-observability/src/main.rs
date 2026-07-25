mod model;

use model::Timeline;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

struct Args {
    trace: PathBuf,
    hotspots: PathBuf,
    snapshot: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let timeline = Timeline::load(&args.trace, &args.hotspots)?;
    if timeline.events.is_empty() {
        return Err("trace has no events".to_string());
    }

    let mut index = timeline.first_interesting_index();
    loop {
        render(&timeline, index, !args.snapshot);
        if args.snapshot {
            break;
        }

        print!("\ncommand> ");
        io::stdout().flush().map_err(|error| error.to_string())?;
        let mut command = String::new();
        io::stdin()
            .read_line(&mut command)
            .map_err(|error| error.to_string())?;
        match command.trim().to_lowercase().as_str() {
            "n" | "" => index = (index + 1).min(timeline.events.len() - 1),
            "p" => index = index.saturating_sub(1),
            "e" => index = next_matching(&timeline, index, |event| event.action == "edit"),
            "v" => index = next_matching(&timeline, index, |event| event.action == "verify"),
            "x" => index = next_matching(&timeline, index, |event| event.is_error),
            "?" | "h" => continue,
            "q" => break,
            _ => continue,
        }
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut trace = None;
    let mut hotspots = None;
    let mut snapshot = false;
    let mut values = env::args().skip(1);

    while let Some(value) = values.next() {
        match value.as_str() {
            "--trace" => trace = values.next().map(PathBuf::from),
            "--hotspots" => hotspots = values.next().map(PathBuf::from),
            "--snapshot" => snapshot = true,
            "-h" | "--help" => return Err(usage()),
            unknown => return Err(format!("unknown argument: {unknown}\n{}", usage())),
        }
    }

    Ok(Args {
        trace: trace.ok_or_else(usage)?,
        hotspots: hotspots.ok_or_else(usage)?,
        snapshot,
    })
}

fn usage() -> String {
    "usage: session-observability --trace <trace.json> --hotspots <sentrux-hotspots.json> [--snapshot]".to_string()
}

fn render(timeline: &Timeline, index: usize, clear: bool) {
    if clear {
        print!("\x1b[2J\x1b[H");
    }
    let event = &timeline.events[index];
    println!("CODE INTEL / SESSION OBSERVABILITY — THROWAWAY PROTOTYPE");
    println!("======================================================");
    println!(
        "session {}  harness {}  event {}/{}  joined targets {}",
        short_id(&timeline.session.id),
        fallback(&timeline.session.harness, "unknown"),
        index + 1,
        timeline.stats.event_count,
        timeline.matched_target_count()
    );
    println!(
        "edited {}  churn files {}  error rate {:.1}%  edits after verify {}",
        timeline.stats.edited,
        timeline.stats.churn_files,
        timeline.stats.error_rate * 100.0,
        timeline.stats.edits_after_last_verify
    );
    println!();
    println!(
        "seq {:>4}  action {:<7} tool {:<22} {}",
        event.seq,
        event.action,
        event.tool,
        if event.is_error { "ERROR" } else { "ok" }
    );
    println!("summary: {}", timeline.redacted_summary(&event.summary));
    println!();
    println!("targets:");
    if event.targets.is_empty() {
        println!("  (none recorded)");
    }
    for target in &event.targets {
        match timeline.hotspot_for(&target.path) {
            Some(hotspot) => println!(
                "  {:<4} {}\n       HOTSPOT max={} avg={:.1} loc={} git-churn={}{}\n       artifact-path: {}",
                target.touch,
                target.path,
                hotspot.max_complexity,
                hotspot.avg_complexity,
                hotspot.loc,
                hotspot.churn,
                if hotspot.dirty { " dirty" } else { "" },
                hotspot.path
            ),
            None => println!("  {:<4} {}\n       structural evidence: unknown", target.touch, target.path),
        }
    }
    println!();
    println!("n next | p previous | e next edit | v next verify | x next error | q quit");
    println!("Raw user-message marks are intentionally not loaded or displayed.");
}

fn next_matching<F>(timeline: &Timeline, current: usize, predicate: F) -> usize
where
    F: Fn(&model::Event) -> bool,
{
    (current + 1..timeline.events.len())
        .find(|index| predicate(&timeline.events[*index]))
        .unwrap_or(current)
}

fn short_id(value: &str) -> &str {
    value.get(..value.len().min(12)).unwrap_or(value)
}

fn fallback<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() {
        fallback
    } else {
        value
    }
}
