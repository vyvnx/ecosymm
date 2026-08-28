//! observations in, action tendencies out.
//!
//! this is the whole of the network's authority. it decides *intent* and
//! nothing more: every rule about whether the intent is affordable, reachable
//! or permitted lives in `actions`, `phenotype` and `interactions`, and the
//! policy has no way to reach around them.

use crate::behavior::actions::Intent;
use ecosym_genetics::{NeuralGenome, INPUTS};

/// one forward pass, decoded. no state, no learning, no reward: the weights an
/// organism was born with are the weights it dies with.
pub fn decide(brain: &NeuralGenome, inputs: &[f32; INPUTS]) -> Intent {
    Intent::decode(brain.forward(inputs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_core::Rng;

    #[test]
    fn different_brains_reach_different_intents_from_the_same_observation() {
        let mut rng = Rng::new(17);
        let inputs = [0.4; INPUTS];
        let a = decide(&NeuralGenome::random(&mut rng), &inputs);
        let b = decide(&NeuralGenome::random(&mut rng), &inputs);
        assert_ne!(a, b);
    }

    #[test]
    fn every_decoded_pressure_is_inside_zero_one() {
        let mut rng = Rng::new(18);
        for _ in 0..200 {
            let brain = NeuralGenome::random(&mut rng);
            let inputs: [f32; INPUTS] = std::array::from_fn(|_| rng.f32());
            let i = decide(&brain, &inputs);
            assert!((0.0..=1.0).contains(&i.seek));
            assert!((0.0..=1.0).contains(&i.breed));
            assert!((0.0..=1.0).contains(&i.rest));
            assert!((-1.0..=1.0).contains(&i.east_west));
            assert!((-1.0..=1.0).contains(&i.north_south));
        }
    }
}
