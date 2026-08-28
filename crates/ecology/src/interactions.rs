//! how organisms interact. for now that is exactly one thing: exploitative
//! competition for a single shared, finite, regrowing resource field. whoever
//! reaches a tile first in the visit order takes from it and everyone after
//! finds less - which is why visit order has to be shuffled every tick.
//!
//! predation, combat, cooperation and migration are deliberately absent.

use crate::phenotype;
use ecosym_genetics::Genes;
use ecosym_world::World;

/// harvest the tile under (x, y), return the energy actually absorbed after
/// the climate penalty
pub fn forage(g: &Genes, world: &mut World, x: f32, y: f32) -> f32 {
    let i = world.idx(x, y);
    let food = world.harvest(i, phenotype::intake(g));
    food * phenotype::climate_fit(g, world.temperature_at(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_forager_leaves_less_for_the_second() {
        let mut world = World::generate(1234, 32, 32);
        let g = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let first = forage(&g, &mut world, 4.0, 4.0);
        let second = forage(&g, &mut world, 4.0, 4.0);
        assert!(second <= first);
        assert!(world.resource_at(world.idx(4.0, 4.0)) >= 0.0);
    }
}
