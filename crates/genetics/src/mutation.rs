//! birth-time mutation. pure: it produces new genes and never touches a living
//! organism's genome.

use crate::neural_genome::clamp;
use crate::{Genes, NeuralGenome};
use ecosym_core::Rng;

pub const MUTATION_RATE: f32 = 0.08;

/// per-trait gaussian drift, clamped to the viable range
pub fn mutate(genes: Genes, rng: &mut Rng) -> Genes {
    Genes {
        speed: genes.speed + rng.normal() * MUTATION_RATE,
        size: genes.size + rng.normal() * MUTATION_RATE,
        metabolism: genes.metabolism + rng.normal() * MUTATION_RATE,
        heat_pref: genes.heat_pref + rng.normal() * MUTATION_RATE * 0.5,
    }
    .clamped()
}

/// chance one weight or bias is perturbed at all at birth
pub const BRAIN_MUTATION_RATE: f32 = 0.05;
/// of the ones that are perturbed, the share that gets the large step. the two
/// together give 95% untouched / 4.5% small / 0.5% large.
pub const BRAIN_BIG_SHARE: f32 = 0.1;
pub const BRAIN_SMALL_STEP: f32 = 0.15;
pub const BRAIN_BIG_STEP: f32 = 0.8;

/// mostly-nothing mutation: a brain that survived is worth keeping, so drift is
/// rare and usually small, with a thin tail large enough to escape a local
/// optimum. weights and biases evolve by the same rule.
pub fn mutate_brain(brain: NeuralGenome, rng: &mut Rng) -> NeuralGenome {
    let mut next = brain;
    for w in next.weights.iter_mut().chain(next.biases.iter_mut()) {
        if rng.f32() >= BRAIN_MUTATION_RATE {
            continue;
        }
        let step = if rng.f32() < BRAIN_BIG_SHARE { BRAIN_BIG_STEP } else { BRAIN_SMALL_STEP };
        *w = clamp(*w + rng.normal() * step);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_stays_in_bounds() {
        let mut rng = Rng::new(7);
        let mut g = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        for _ in 0..10_000 {
            g = mutate(g, &mut rng);
            assert!(g.in_bounds(), "{g:?}");
        }
    }

    #[test]
    fn mutation_is_deterministic_for_a_seed() {
        let g = Genes { speed: 1.0, size: 1.0, metabolism: 1.0, heat_pref: 0.5 };
        let run = || {
            let mut rng = Rng::new(3);
            (0..50).fold(g, |acc, _| mutate(acc, &mut rng))
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn brain_mutation_touches_about_one_gene_in_twenty() {
        let mut rng = Rng::new(5);
        let base = NeuralGenome::random(&mut rng);
        let (mut touched, mut total) = (0, 0);
        for _ in 0..200 {
            let child = mutate_brain(base, &mut rng);
            touched += base.genes().zip(child.genes()).filter(|(a, b)| a != b).count();
            total += base.genes().count();
        }
        let rate = touched as f32 / total as f32;
        assert!((0.03..0.07).contains(&rate), "mutated {rate} of all genes");
    }

    #[test]
    fn brain_mutation_stays_in_bounds_and_is_deterministic() {
        let run = || {
            let mut rng = Rng::new(9);
            let base = NeuralGenome::random(&mut rng);
            (0..500).fold(base, |acc, _| mutate_brain(acc, &mut rng))
        };
        let drifted = run();
        assert_eq!(drifted, run());
        assert!(drifted.in_bounds() && drifted.is_finite());
    }

    #[test]
    fn brain_mutation_actually_moves_a_brain_over_many_births() {
        let mut rng = Rng::new(21);
        let base = NeuralGenome::random(&mut rng);
        let drifted = (0..300).fold(base, |acc, _| mutate_brain(acc, &mut rng));
        assert!(drifted.distance(&base) > 0.05, "300 births barely moved the brain");
    }
}
