//! `cargo run --release -- --seed 1234 --population-per-species 500 --epochs 500`

use clap::Parser;
use ecosym_core::SimConfig;
use ecosym_genetics::Genes;
use ecosym_replay::Recorder;
use ecosym_simulation::{
    twin_blueprints, BehaviorStats, RunOutcome, Simulation, SpeciesResult, Winner,
};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "ecosym", about = "deterministic ecosystem simulation")]
struct Args {
    #[arg(long, default_value_t = SimConfig::default().seed)]
    seed: u64,
    /// founders spawned for each species in the scenario
    #[arg(long, default_value_t = SimConfig::default().population_per_species)]
    population_per_species: usize,
    /// batches of ticks to run. simulation time, not biological generations.
    #[arg(long, default_value_t = SimConfig::default().epochs)]
    epochs: usize,
    /// run the controlled experiment instead: two species with identical
    /// bodies, so only their evolved policies can differ
    #[arg(long)]
    twins: bool,
}

fn main() {
    let args = Args::parse();
    let cfg = SimConfig {
        seed: args.seed,
        population_per_species: args.population_per_species,
        epochs: args.epochs,
        ..Default::default()
    };

    let mut sim = if args.twins {
        Simulation::cpu_with(cfg.clone(), &twin_blueprints())
    } else {
        Simulation::cpu(cfg.clone())
    };
    let mut rec = Recorder::new(cfg.clone(), sim.engine_id());
    let names: Vec<String> = sim.state.species.iter().map(|s| s.name().to_string()).collect();

    let world = sim.state.world.summary();
    println!("engine: {}", sim.engine_id());
    println!(
        "world: {}x{}, {} habitable tiles, {:.0} initial biomass, mean temperature {:.2}",
        world.width,
        world.height,
        world.habitable_tiles,
        world.initial_biomass,
        world.mean_temperature
    );
    for species in &sim.state.species {
        println!(
            "{}: spawned {}, founder genes {}",
            species.name(),
            species.population().len(),
            genes(species.founder_genes())
        );
    }
    println!("seed {}, {} epochs of {} ticks", cfg.seed, cfg.epochs, cfg.ticks_per_epoch);

    print!("\n{:>6} {:>9} {:>11}", "epoch", "total", "biomass");
    for name in &names {
        print!(" {name:>12}");
    }
    println!();

    let every = (cfg.epochs / 20).max(1);
    let started = Instant::now();

    for e in 1..=cfg.epochs {
        let report = sim.advance_epoch().expect("cpu engine cannot fail");
        if e == 1 || e % every == 0 || e == cfg.epochs {
            print!("{:>6} {:>9} {:>11.1}", report.epoch, report.population, report.biomass);
            for s in &report.species {
                print!(" {:>12}", s.population);
            }
            println!();
        }
        let extinct = report.population == 0;
        rec.push(report);
        if extinct {
            println!("\ntotal extinction at epoch {e}");
            break;
        }
    }

    let elapsed = started.elapsed();
    let outcome = sim.outcome();
    println!("\nresult after {} epochs", outcome.epochs);
    for s in &outcome.species {
        println!(
            "{}: initial {}, final {}, change {:+} ({:+.1}%), births {}, deaths {}",
            s.name,
            s.initial,
            s.final_population,
            s.change(),
            s.change_pct(),
            s.births,
            s.deaths
        );
        println!("  genes    {}", gene_changes(&s.founder_genes, &s.final_genes));
        println!("  behavior {}", behavior_changes(&s.founder_behavior, &s.final_behavior));
        println!(
            "  brain drift {:.4} per neural gene from the founder policy, mean energy {:.2}",
            s.brain_drift, s.final_energy
        );
    }
    println!("{}", winner_line(&outcome));
    // a winner is the least-shrunken species, nothing more
    println!("winning is relative: it does not mean the winner is ecologically healthy.");
    if sim.ceiling_bound() {
        println!("note: the population safety ceiling refused births during this run.");
    }

    println!("\nreplay digest: {}", rec.digest_hex());
    println!("epochs recorded: {}", rec.epochs());
    println!("wall clock: {:.2}s", elapsed.as_secs_f64());
}

fn genes(g: &Genes) -> String {
    format!(
        "speed {:.3} size {:.3} metabolism {:.3} heat_pref {:.3}",
        g.speed, g.size, g.metabolism, g.heat_pref
    )
}

fn gene_changes(from: &Genes, to: &Genes) -> String {
    [
        ("speed", from.speed, to.speed),
        ("size", from.size, to.size),
        ("metabolism", from.metabolism, to.metabolism),
        ("heat_pref", from.heat_pref, to.heat_pref),
    ]
    .iter()
    .map(|(name, a, b)| format!("{name} {a:.3} -> {b:.3}"))
    .collect::<Vec<_>>()
    .join("   ")
}

/// behavioural means, first recorded epoch to last. descriptive: nothing in the
/// simulation ever reads these back.
fn behavior_changes(from: &BehaviorStats, to: &BehaviorStats) -> String {
    [
        ("movement/tick", from.movement, to.movement),
        ("food seeking", from.food_seeking, to.food_seeking),
        ("reproduction intent", from.reproduction, to.reproduction),
        ("rest tendency", from.resting, to.resting),
        ("competitor exposure", from.competitor_exposure, to.competitor_exposure),
    ]
    .iter()
    .map(|(name, a, b)| format!("{name} {a:.3} -> {b:.3}"))
    .collect::<Vec<_>>()
    .join("   ")
}

fn winner_line(outcome: &RunOutcome) -> String {
    let name = |id: u32| {
        outcome
            .species
            .iter()
            .find(|s: &&SpeciesResult| s.id == id)
            .map_or_else(|| format!("species {id}"), |s| s.name.clone())
    };
    match &outcome.winner {
        Winner::Species(id) => format!("winner: {}", name(*id)),
        Winner::Tie(ids) => {
            format!(
                "winner: tie between {}",
                ids.iter().map(|i| name(*i)).collect::<Vec<_>>().join(", ")
            )
        }
        Winner::None => "winner: none - every species went extinct".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: u32, name: &str, final_population: usize) -> SpeciesResult {
        SpeciesResult {
            id,
            name: name.to_string(),
            initial: 500,
            final_population,
            births: 0,
            deaths: 0,
            founder_genes: Genes::default(),
            final_genes: Genes::default(),
            final_energy: 0.0,
            founder_behavior: BehaviorStats::default(),
            final_behavior: BehaviorStats::default(),
            brain_drift: 0.0,
        }
    }

    fn outcome(species: Vec<SpeciesResult>) -> RunOutcome {
        RunOutcome { winner: ecosym_simulation::statistics::winner(&species), epochs: 10, species }
    }

    #[test]
    fn the_winner_line_names_species_ties_and_extinction() {
        let a = outcome(vec![result(0, "Species A", 300), result(1, "Species B", 100)]);
        assert_eq!(winner_line(&a), "winner: Species A");

        let tie = outcome(vec![result(0, "Species A", 200), result(1, "Species B", 200)]);
        assert_eq!(winner_line(&tie), "winner: tie between Species A, Species B");

        let gone = outcome(vec![result(0, "Species A", 0), result(1, "Species B", 0)]);
        assert_eq!(winner_line(&gone), "winner: none - every species went extinct");
    }

    #[test]
    fn behavior_changes_show_both_ends() {
        let from = BehaviorStats {
            movement: 0.81,
            food_seeking: 0.63,
            reproduction: 0.52,
            resting: 0.14,
            competitor_exposure: 0.2,
        };
        let to = BehaviorStats {
            movement: 0.52,
            food_seeking: 0.82,
            reproduction: 0.39,
            resting: 0.61,
            competitor_exposure: 0.3,
        };
        let line = behavior_changes(&from, &to);
        assert!(line.contains("movement/tick 0.810 -> 0.520"), "{line}");
        assert!(line.contains("rest tendency 0.140 -> 0.610"), "{line}");
        assert!(line.contains("food seeking 0.630 -> 0.820"), "{line}");
    }

    #[test]
    fn gene_changes_show_both_ends() {
        let from = Genes { speed: 1.3, size: 1.0, metabolism: 1.2, heat_pref: 0.62 };
        let to = Genes { speed: 1.8, size: 0.9, metabolism: 0.2, heat_pref: 0.6 };
        let line = gene_changes(&from, &to);
        assert!(line.contains("speed 1.300 -> 1.800"), "{line}");
        assert!(line.contains("metabolism 1.200 -> 0.200"), "{line}");
    }
}
