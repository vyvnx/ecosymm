//! package a: how often does each betting outcome win?
//!
//! samples default-config two-species runs and reports, for a range of
//! candidate coexistence margins, how often the market would settle on
//! species 1, coexistence, species 2 or void.
//!
//! ```bash
//! cargo run --release -p ecosym-cli --bin calibrate -- --runs 1000 > table.txt
//! ```
//!
//! the sample is a contiguous seed range and the rows are sorted by seed, so
//! the output does not depend on how many threads ran it.

use clap::Parser;
use ecosym_core::SimConfig;
use ecosym_replay::Recorder;
use ecosym_simulation::Simulation;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Parser)]
#[command(name = "calibrate", about = "sample betting outcomes over many seeds")]
struct Args {
    /// seeds to sample, starting at `--first-seed`
    #[arg(long, default_value_t = 1000)]
    runs: u64,
    #[arg(long, default_value_t = 1)]
    first_seed: u64,
    #[arg(long, default_value_t = 8)]
    threads: usize,
    /// candidate margins for `abs(ln(score_a / score_b)) <= margin`
    #[arg(long, value_delimiter = ',', default_values_t = [0.05, 0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.50])]
    margins: Vec<f64>,
}

struct Row {
    seed: u64,
    initial: [usize; 2],
    final_pop: [usize; 2],
    digest: String,
}

fn main() {
    let args = Args::parse();
    let seeds: Vec<u64> = (0..args.runs).map(|i| args.first_seed + i).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);

    let mut rows: Vec<Row> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..args.threads.max(1))
            .map(|_| {
                let (seeds, next, done) = (&seeds, &next, &done);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&seed) = seeds.get(i) else { return out };
                        out.push(sample(seed));
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if n % 10 == 0 {
                            eprintln!("{n}/{} runs", seeds.len());
                        }
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("sample thread")).collect()
    });
    rows.sort_by_key(|r| r.seed);

    println!("seed,initial_a,final_a,initial_b,final_b,score_a,score_b,digest");
    for r in &rows {
        println!(
            "{},{},{},{},{},{:.6},{:.6},{}",
            r.seed,
            r.initial[0],
            r.final_pop[0],
            r.initial[1],
            r.final_pop[1],
            score(r, 0),
            score(r, 1),
            r.digest
        );
    }

    let void = rows.iter().filter(|r| r.final_pop == [0, 0]).count();
    let one_sided =
        rows.iter().filter(|r| r.final_pop != [0, 0] && r.final_pop.contains(&0)).count();
    println!();
    println!(
        "runs {}, seeds {}..={}",
        rows.len(),
        args.first_seed,
        args.first_seed + args.runs - 1
    );
    println!("void (both extinct)      {void} ({:.1}%)", pct(void, rows.len()));
    println!("single survivor          {one_sided} ({:.1}%)", pct(one_sided, rows.len()));
    println!();
    println!(
        "{:>8} {:>10} {:>14} {:>10}   coexistence share of non-void",
        "margin", "species a", "coexistence", "species b"
    );
    let non_void = rows.len() - void;
    for &margin in &args.margins {
        let mut counts = [0usize; 3];
        for r in &rows {
            if r.final_pop == [0, 0] {
                continue;
            }
            counts[classify(r, margin)] += 1;
        }
        println!(
            "{margin:>8.2} {:>10} {:>14} {:>10}   {:.1}%",
            counts[0],
            counts[1],
            counts[2],
            pct(counts[1], non_void)
        );
    }
}

fn sample(seed: u64) -> Row {
    let cfg = SimConfig { seed, ..SimConfig::default() };
    let mut sim = Simulation::cpu(cfg.clone());
    let mut rec = Recorder::new(cfg.clone(), sim.engine_id());
    for _ in 0..cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        let extinct = report.population == 0;
        rec.push(report);
        if extinct {
            break;
        }
    }
    let outcome = sim.outcome();
    Row {
        seed,
        initial: [outcome.species[0].initial, outcome.species[1].initial],
        final_pop: [outcome.species[0].final_population, outcome.species[1].final_population],
        digest: rec.digest_hex(),
    }
}

fn score(r: &Row, i: usize) -> f64 {
    r.final_pop[i] as f64 / r.initial[i] as f64
}

/// 0 = species a, 1 = coexistence, 2 = species b. callers filter void first.
fn classify(r: &Row, margin: f64) -> usize {
    match (r.final_pop[0], r.final_pop[1]) {
        (0, _) => 2,
        (_, 0) => 0,
        _ => {
            let gap = (score(r, 0) / score(r, 1)).ln();
            if gap.abs() <= margin {
                1
            } else if gap > 0.0 {
                0
            } else {
                2
            }
        }
    }
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * n as f64 / total as f64
    }
}
