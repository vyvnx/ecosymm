//! how two parents' genes combine. pure, like mutation.

use crate::{Genes, NeuralGenome};
use ecosym_core::Rng;

/// uniform crossover, one coin flip per trait
pub fn recombine(a: &Genes, b: &Genes, rng: &mut Rng) -> Genes {
    Genes {
        speed: pick(a.speed, b.speed, rng),
        size: pick(a.size, b.size, rng),
        metabolism: pick(a.metabolism, b.metabolism, rng),
        heat_pref: pick(a.heat_pref, b.heat_pref, rng),
    }
}

/// the same rule one layer down: one coin flip per weight and per bias, so a
/// child's brain is built only out of alleles its two parents actually carried.
pub fn recombine_brain(a: &NeuralGenome, b: &NeuralGenome, rng: &mut Rng) -> NeuralGenome {
    let mut child = *a;
    for (i, w) in child.weights.iter_mut().enumerate() {
        *w = pick(a.weights[i], b.weights[i], rng);
    }
    for (i, w) in child.biases.iter_mut().enumerate() {
        *w = pick(a.biases[i], b.biases[i], rng);
    }
    child
}

fn pick(a: f32, b: f32, rng: &mut Rng) -> f32 {
    if rng.f32() < 0.5 {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossover_only_copies_parent_alleles() {
        let mut rng = Rng::new(3);
        let a = Genes { speed: 0.5, size: 0.6, metabolism: 0.7, heat_pref: 0.2 };
        let b = Genes { speed: 1.5, size: 1.6, metabolism: 1.7, heat_pref: 0.8 };
        for _ in 0..100 {
            let c = recombine(&a, &b, &mut rng);
            assert!(c.speed == a.speed || c.speed == b.speed);
            assert!(c.size == a.size || c.size == b.size);
            assert!(c.metabolism == a.metabolism || c.metabolism == b.metabolism);
            assert!(c.heat_pref == a.heat_pref || c.heat_pref == b.heat_pref);
        }
    }

    #[test]
    fn brain_crossover_only_copies_parent_alleles_and_mixes_both() {
        let mut rng = Rng::new(3);
        let a = NeuralGenome::random(&mut rng);
        let b = NeuralGenome::random(&mut rng);
        let mut from_a = 0;
        let mut from_b = 0;
        for _ in 0..20 {
            let c = recombine_brain(&a, &b, &mut rng);
            for ((x, y), z) in a.genes().zip(b.genes()).zip(c.genes()) {
                assert!(z == x || z == y, "child carries an allele neither parent had");
                from_a += usize::from(z == x);
                from_b += usize::from(z == y);
            }
        }
        assert!(from_a > 0 && from_b > 0, "crossover copied only one parent");
    }

    #[test]
    fn brain_crossover_of_two_identical_parents_changes_nothing() {
        let mut rng = Rng::new(4);
        let a = NeuralGenome::random(&mut rng);
        assert_eq!(recombine_brain(&a, &a, &mut rng), a);
    }
}
