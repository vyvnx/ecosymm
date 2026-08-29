//! read-only render extraction: the smallest copy of live state a viewer needs.
//!
//! this sits beside `SimulationState`, not inside `EpochEngine`. it is a
//! cpu-era adapter - a future gpu-resident backend may produce the same wire
//! data straight from device buffers without ever materialising these types,
//! which is exactly why the engine contract does not mention rendering.
//!
//! nothing here may mutate state, reorder a population, touch a random stream
//! or reach the replay digest. no genome, no neural weight, no colour: a
//! viewer gets position, identity and energy, and that is all.

use crate::state::SimulationState;
use ecosym_world::World;
use std::fmt;

/// the extractor refuses rather than truncates. every one of these means the
/// state cannot be described on a wire that declares `u32` counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderError {
    /// a dimension, cell count, organism count or epoch past what `u32` holds
    TooLarge(&'static str, u64),
    /// a world field that is not exactly `width * height` long
    FieldLength { expected: usize, found: usize },
    /// a nan or an infinity reached the viewer, which is a simulation bug
    NonFinite(&'static str),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::TooLarge(what, n) => write!(f, "{what} does not fit a u32: {n}"),
            RenderError::FieldLength { expected, found } => {
                write!(f, "world field is {found} long, expected {expected}")
            }
            RenderError::NonFinite(what) => write!(f, "{what} is not finite"),
        }
    }
}

impl std::error::Error for RenderError {}

/// the static half of a run: extracted once, never again.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderWorld {
    pub width: u32,
    pub height: u32,
    /// row-major, `width * height` long
    pub fertility: Vec<f32>,
    /// row-major, `width * height` long
    pub temperature: Vec<f32>,
}

/// one organism as a viewer sees it. `species_id` comes from the owning
/// species, never from a field on `Organism` that could drift out of sync.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOrganism {
    pub id: u64,
    pub species_id: u32,
    /// the raw simulated coordinate, uncanonicalised. wire encoding normalises
    /// it for display; the simulation keeps whatever it had.
    pub x: f32,
    pub y: f32,
    pub energy: f32,
}

/// the moving half: what changed since the last time anybody looked.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSnapshot {
    pub epoch: u32,
    /// standing resource per tile, row-major. fullness is a wire concern - it
    /// needs the capacity field, which lives in `RenderWorld`.
    pub resources: Vec<f32>,
    pub organisms: Vec<RenderOrganism>,
}

impl RenderWorld {
    pub fn extract(world: &World) -> Result<RenderWorld, RenderError> {
        let width = fits("width", world.width())?;
        let height = fits("height", world.height())?;
        let cells = cell_count(width, height)? as usize;
        check_len(cells, world.fertility().len())?;
        check_len(cells, world.temperature().len())?;
        finite("fertility", world.fertility())?;
        finite("temperature", world.temperature())?;
        Ok(RenderWorld {
            width,
            height,
            fertility: world.fertility().to_vec(),
            temperature: world.temperature().to_vec(),
        })
    }
}

impl RenderSnapshot {
    /// organisms are visited in species order and then population order. the
    /// browser reconciles by id and must not depend on that, but a stable walk
    /// keeps the wire diffable by eye when something looks wrong.
    pub fn extract(state: &SimulationState) -> Result<RenderSnapshot, RenderError> {
        let epoch = fits("epoch", state.time.epoch.0)?;
        let world = &state.world;
        let cells =
            cell_count(fits("width", world.width())?, fits("height", world.height())?)? as usize;
        check_len(cells, world.resources().len())?;
        finite("resources", world.resources())?;

        let mut organisms = Vec::with_capacity(state.population());
        for species in &state.species {
            let species_id = species.id().get();
            for o in species.population().organisms() {
                if !(o.x.is_finite() && o.y.is_finite() && o.energy.is_finite()) {
                    return Err(RenderError::NonFinite("organism"));
                }
                organisms.push(RenderOrganism {
                    id: o.id().get(),
                    species_id,
                    x: o.x,
                    y: o.y,
                    energy: o.energy,
                });
            }
        }
        fits("organism count", organisms.len())?;

        Ok(RenderSnapshot { epoch, resources: world.resources().to_vec(), organisms })
    }
}

fn fits(what: &'static str, n: usize) -> Result<u32, RenderError> {
    u32::try_from(n).map_err(|_| RenderError::TooLarge(what, n as u64))
}

fn cell_count(width: u32, height: u32) -> Result<u32, RenderError> {
    width
        .checked_mul(height)
        .ok_or(RenderError::TooLarge("cell count", u64::from(width) * u64::from(height)))
}

fn check_len(expected: usize, found: usize) -> Result<(), RenderError> {
    if expected == found {
        Ok(())
    } else {
        Err(RenderError::FieldLength { expected, found })
    }
}

fn finite(what: &'static str, values: &[f32]) -> Result<(), RenderError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(RenderError::NonFinite(what))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Simulation;
    use ecosym_core::SimConfig;
    use ecosym_ecology::SpeciesBlueprint;
    use ecosym_genetics::Genes;

    fn small() -> SimConfig {
        SimConfig {
            seed: 1234,
            population_per_species: 20,
            epochs: 5,
            width: 32,
            height: 24,
            ticks_per_epoch: 5,
        }
    }

    fn blueprints(n: usize) -> Vec<SpeciesBlueprint> {
        (0..n)
            .map(|i| SpeciesBlueprint {
                name: format!("S{i}"),
                genes: Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 },
                gene_spread: 0.1,
            })
            .collect()
    }

    #[test]
    fn the_world_arrives_row_major_and_the_right_size() {
        let sim = Simulation::cpu(small());
        let world = RenderWorld::extract(&sim.state.world).unwrap();
        assert_eq!((world.width, world.height), (32, 24));
        assert_eq!(world.fertility.len(), 32 * 24);
        assert_eq!(world.temperature.len(), 32 * 24);
        // byte-identical to the simulation's own fields, in the same order
        assert_eq!(world.fertility, sim.state.world.fertility());
        assert_eq!(world.temperature, sim.state.world.temperature());
    }

    /// a 1x1 world is still a world
    #[test]
    fn the_smallest_world_extracts() {
        let cfg = SimConfig { width: 1, height: 1, population_per_species: 2, ..small() };
        let sim = Simulation::cpu(cfg);
        let world = RenderWorld::extract(&sim.state.world).unwrap();
        assert_eq!((world.width, world.height, world.fertility.len()), (1, 1, 1));
        let snap = RenderSnapshot::extract(&sim.state).unwrap();
        assert_eq!(snap.resources.len(), 1);
    }

    #[test]
    fn species_ownership_decides_the_species_id_and_the_walk_order() {
        for n in [0usize, 1, 2, 3] {
            let sim = Simulation::cpu_with(small(), &blueprints(n));
            let snap = RenderSnapshot::extract(&sim.state).unwrap();
            assert_eq!(snap.organisms.len(), n * small().population_per_species);

            let mut at = 0;
            for species in &sim.state.species {
                for o in species.population().organisms() {
                    let r = snap.organisms[at];
                    assert_eq!(r.species_id, species.id().get());
                    assert_eq!(r.id, o.id().get());
                    assert_eq!((r.x, r.y, r.energy), (o.x, o.y, o.energy));
                    at += 1;
                }
            }
            assert_eq!(at, snap.organisms.len());
        }
    }

    #[test]
    fn a_world_with_no_species_still_extracts_an_empty_snapshot() {
        let sim = Simulation::cpu_with(small(), &blueprints(0));
        let snap = RenderSnapshot::extract(&sim.state).unwrap();
        assert!(snap.organisms.is_empty());
        assert_eq!(snap.epoch, 0);
        assert_eq!(snap.resources.len(), 32 * 24);
    }

    #[test]
    fn extinction_extracts_as_an_empty_population_not_an_error() {
        let mut sim = Simulation::cpu_with(small(), &blueprints(1));
        for species in &mut sim.state.species {
            for i in 0..species.population().len() {
                species.population_mut().get_mut(i).unwrap().energy = -1.0;
            }
            species.population_mut().retain_living();
        }
        let snap = RenderSnapshot::extract(&sim.state).unwrap();
        assert!(snap.organisms.is_empty());
    }

    /// ids are minted once and never reused, so a survivor keeps the identity
    /// the browser reconciles it by
    #[test]
    fn survivors_keep_their_id_across_an_epoch() {
        let mut sim = Simulation::cpu(small());
        let before = RenderSnapshot::extract(&sim.state).unwrap();
        sim.advance_epoch().unwrap();
        let after = RenderSnapshot::extract(&sim.state).unwrap();

        assert_eq!(after.epoch, before.epoch + 1);
        let survivors: Vec<u64> = after
            .organisms
            .iter()
            .map(|o| o.id)
            .filter(|id| before.organisms.iter().any(|b| b.id == *id))
            .collect();
        assert!(!survivors.is_empty(), "nothing survived, so this proves nothing");
        for id in survivors {
            let b = before.organisms.iter().find(|o| o.id == id).unwrap();
            let a = after.organisms.iter().find(|o| o.id == id).unwrap();
            assert_eq!(a.species_id, b.species_id, "an organism changed species");
        }
    }

    #[test]
    fn every_extracted_value_is_finite() {
        let mut sim = Simulation::cpu(small());
        for _ in 0..5 {
            sim.advance_epoch().unwrap();
            let world = RenderWorld::extract(&sim.state.world).unwrap();
            let snap = RenderSnapshot::extract(&sim.state).unwrap();
            assert!(world.fertility.iter().all(|v| v.is_finite()));
            assert!(world.temperature.iter().all(|v| v.is_finite()));
            assert!(snap.resources.iter().all(|v| v.is_finite()));
            assert!(snap
                .organisms
                .iter()
                .all(|o| o.x.is_finite() && o.y.is_finite() && o.energy.is_finite()));
        }
    }

    /// a nan in the state is a bug the viewer reports rather than paints
    #[test]
    fn a_non_finite_organism_is_an_error() {
        let mut sim = Simulation::cpu(small());
        sim.state.species[0].population_mut().get_mut(0).unwrap().x = f32::NAN;
        assert_eq!(RenderSnapshot::extract(&sim.state), Err(RenderError::NonFinite("organism")));
    }

    /// raw coordinates cross the seam untouched. normalising here would mean
    /// the renderer had edited simulation state, which is the one thing this
    /// module must never do.
    #[test]
    fn coordinates_are_extracted_raw_on_both_sides_of_the_seam() {
        let mut sim = Simulation::cpu(small());
        {
            let p = sim.state.species[0].population_mut();
            p.get_mut(0).unwrap().x = -0.5;
            p.get_mut(0).unwrap().y = 24.25;
            p.get_mut(1).unwrap().x = 32.75;
            p.get_mut(1).unwrap().y = -3.0;
        }
        let snap = RenderSnapshot::extract(&sim.state).unwrap();
        assert_eq!((snap.organisms[0].x, snap.organisms[0].y), (-0.5, 24.25));
        assert_eq!((snap.organisms[1].x, snap.organisms[1].y), (32.75, -3.0));
    }

    /// the acceptance property: looking changes nothing. extraction runs
    /// before and after every epoch and the reports come out identical.
    #[test]
    fn extracting_every_epoch_does_not_change_the_run() {
        let plain = {
            let mut sim = Simulation::cpu(small());
            let reports: Vec<_> = (0..5).map(|_| sim.advance_epoch().unwrap()).collect();
            (reports, sim.outcome())
        };
        let watched = {
            let mut sim = Simulation::cpu(small());
            let mut reports = Vec::new();
            for _ in 0..5 {
                RenderSnapshot::extract(&sim.state).unwrap();
                reports.push(sim.advance_epoch().unwrap());
                RenderSnapshot::extract(&sim.state).unwrap();
                RenderWorld::extract(&sim.state.world).unwrap();
            }
            (reports, sim.outcome())
        };
        assert_eq!(plain, watched);
    }

    #[test]
    fn counts_that_do_not_fit_the_wire_are_errors_not_truncations() {
        assert_eq!(
            cell_count(70_000, 70_000),
            Err(RenderError::TooLarge("cell count", 4_900_000_000))
        );
        assert_eq!(cell_count(65_535, 65_535).map(|c| c as u64), Ok(4_294_836_225));
        assert_eq!(check_len(4, 5), Err(RenderError::FieldLength { expected: 4, found: 5 }));
    }
}
