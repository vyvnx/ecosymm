//! the pantheon founder acceptance gate and the compatibility gate, over a
//! matched seed range.
//!
//! ```bash
//! cargo run --release -p ecosym-cli --bin pantheon -- --mode solo --runs 1000 > solo.csv
//! cargo run --release -p ecosym-cli --bin pantheon -- --mode trio --runs 1000 > trio.csv
//! ```
//!
//! the sample is a contiguous seed range and the rows are sorted by seed, so
//! the output does not depend on how many threads ran it.

use clap::Parser;
use ecosym_core::SimConfig;
use ecosym_ecology::SpeciesBlueprint;
use ecosym_replay::Recorder;
use ecosym_simulation::{pantheon_blueprints, Simulation};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// the epoch the trio checks survival and dominance at
const CENSUS_EPOCH: usize = 50;

#[derive(Parser)]
#[command(name = "pantheon", about = "sample pantheon founder outcomes over many seeds")]
struct Args {
    /// seeds to sample, starting at `--first-seed`
    #[arg(long, default_value_t = 1000)]
    runs: u64,
    #[arg(long, default_value_t = 1)]
    first_seed: u64,
    #[arg(long, default_value_t = 8)]
    threads: usize,
    /// `solo` runs each house on its own world; `trio` runs all three together
    #[arg(long, default_value = "trio")]
    mode: String,
    #[arg(long, default_value_t = 500)]
    epochs: usize,
    #[arg(long, default_value_t = 500)]
    population_per_species: usize,
}

struct Row {
    seed: u64,
    house: String,
    initial: usize,
    at_census: usize,
    final_pop: usize,
    births: usize,
    /// total population of the run at `CENSUS_EPOCH`, for the dominance check
    census_total: usize,
    digest: String,
    seconds: f64,
}

fn main() {
    let args = Args::parse();
    let solo = match args.mode.as_str() {
        "solo" => true,
        "trio" => false,
        other => panic!("--mode must be solo or trio, not {other}"),
    };
    let seeds: Vec<u64> = (0..args.runs).map(|i| args.first_seed + i).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);

    let mut rows: Vec<Row> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..args.threads.max(1))
            .map(|_| {
                let (seeds, next, done, args) = (&seeds, &next, &done, &args);
                scope.spawn(move || {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&seed) = seeds.get(i) else { return out };
                        out.extend(sample(seed, solo, args));
                        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                        if n % 10 == 0 {
                            eprintln!("{n}/{} seeds", seeds.len());
                        }
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("sample thread")).collect()
    });
    rows.sort_by(|a, b| (a.seed, &a.house).cmp(&(b.seed, &b.house)));

    println!("seed,house,initial,at_census,final,births,census_total,digest,seconds");
    for r in &rows {
        println!(
            "{},{},{},{},{},{},{},{},{:.2}",
            r.seed,
            r.house,
            r.initial,
            r.at_census,
            r.final_pop,
            r.births,
            r.census_total,
            r.digest,
            r.seconds
        );
    }
    summarise(&rows, solo, args.epochs);
}

/// one seed. `solo` gives each house its own world; otherwise all three share
/// one, which is the run the gate's survival and dominance bars are about.
fn sample(seed: u64, solo: bool, args: &Args) -> Vec<Row> {
    let cfg = SimConfig {
        seed,
        population_per_species: args.population_per_species,
        epochs: args.epochs,
        ..Default::default()
    };
    let houses = pantheon_blueprints();
    if solo {
        houses.iter().flat_map(|h| run_one(&cfg, std::slice::from_ref(h))).collect()
    } else {
        run_one(&cfg, &houses)
    }
}

fn run_one(cfg: &SimConfig, scenario: &[SpeciesBlueprint]) -> Vec<Row> {
    let started = Instant::now();
    let mut sim = Simulation::cpu_with(cfg.clone(), scenario);
    let mut rec = Recorder::new(cfg.clone(), sim.engine_id());
    let mut at_census = vec![0usize; scenario.len()];

    for e in 1..=cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        if e == CENSUS_EPOCH {
            at_census = report.species.iter().map(|s| s.population).collect();
        }
        let extinct = report.population == 0;
        rec.push(report);
        if extinct {
            break;
        }
    }

    let seconds = started.elapsed().as_secs_f64();
    let digest = rec.digest_hex();
    let census_total: usize = at_census.iter().sum();
    let outcome = sim.outcome();
    outcome
        .species
        .iter()
        .enumerate()
        .map(|(i, s)| Row {
            seed: cfg.seed,
            house: s.name.clone(),
            initial: s.initial,
            at_census: at_census.get(i).copied().unwrap_or(0),
            final_pop: s.final_population,
            births: s.births,
            census_total,
            digest: digest.clone(),
            seconds,
        })
        .collect()
}

/// the four gate bars, printed as csv comments so a reader of the file sees the
/// verdict next to the data it came from
fn summarise(rows: &[Row], solo: bool, epochs: usize) {
    let mut houses: Vec<&str> = rows.iter().map(|r| r.house.as_str()).collect();
    houses.sort_unstable();
    houses.dedup();
    let seeds = rows.len() / houses.len().max(1);
    let pct = |n: usize| 100.0 * n as f64 / seeds.max(1) as f64;

    println!("# gate: {seeds} seeds, {epochs} epochs, mode {}", if solo { "solo" } else { "trio" });
    for h in &houses {
        let mine: Vec<&Row> = rows.iter().filter(|r| r.house == *h).collect();
        let survived = mine.iter().filter(|r| r.final_pop > 0).count();
        let at_50 = mine.iter().filter(|r| r.at_census > 0).count();
        let dominant = mine
            .iter()
            .filter(|r| r.census_total > 0 && r.at_census * 5 > r.census_total * 4)
            .count();
        let won = mine
            .iter()
            .filter(|r| {
                let peers: Vec<&Row> = rows.iter().filter(|o| o.seed == r.seed).collect();
                r.final_pop > 0 && peers.iter().all(|o| o.final_pop <= r.final_pop)
            })
            .count();
        println!(
            "# gate: {h}: survives_run {:.1}% (bar 95 solo), alive_at_{CENSUS_EPOCH} {:.1}% (bar 80 trio), \
             over_80pct_at_{CENSUS_EPOCH} {:.1}% (bar <=50 trio), top_house {:.1}% (bar >=10 trio)",
            pct(survived),
            pct(at_50),
            pct(dominant),
            pct(won)
        );
    }
}
