//! observations in, action tendencies out.
//!
//! this is the whole of the network's authority. it decides *intent* and
//! nothing more: every rule about whether the intent is affordable, reachable
//! or permitted lives in `actions`, `phenotype` and `interactions`, and the
//! policy has no way to reach around them.

use crate::behavior::actions::Intent;
use ecosym_genetics::{NeuralGenome, HIDDEN, INPUTS};

/// one forward pass, decoded. the weights an organism was born with are the
/// weights it dies with; `memory` is the only thing a tick may change, and it
/// is the organism's, not the genome's.
pub fn decide(brain: &NeuralGenome, inputs: &[f32; INPUTS], memory: &mut [f32; HIDDEN]) -> Intent {
    Intent::decode(brain.forward(inputs, memory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_core::Rng;

    #[test]
    fn different_brains_reach_different_intents_from_the_same_observation() {
        let mut rng = Rng::new(17);
        let inputs = [0.4; INPUTS];
        let a = decide(&NeuralGenome::random(&mut rng), &inputs, &mut [0.0; HIDDEN]);
        let b = decide(&NeuralGenome::random(&mut rng), &inputs, &mut [0.0; HIDDEN]);
        assert_ne!(a, b);
    }

    /// the point of the recurrent layer: the same brain seeing the same thing
    /// twice does not have to answer the same way, because the second time it
    /// has already seen it once.
    #[test]
    fn the_same_observation_reads_differently_once_it_has_been_seen_before() {
        let brain = NeuralGenome::random(&mut Rng::new(19));
        let inputs = [0.4; INPUTS];
        let mut memory = [0.0; HIDDEN];
        let first = decide(&brain, &inputs, &mut memory);
        let settled = memory;
        let second = decide(&brain, &inputs, &mut memory);
        assert_ne!(first, second, "the hidden state did nothing");
        assert_ne!(settled, memory, "the hidden state did not advance");

        // and a fresh organism with the same brain starts over from zero
        assert_eq!(first, decide(&brain, &inputs, &mut [0.0; HIDDEN]));
    }

    #[test]
    fn every_decoded_pressure_is_inside_zero_one() {
        let mut rng = Rng::new(18);
        for _ in 0..200 {
            let brain = NeuralGenome::random(&mut rng);
            let inputs: [f32; INPUTS] = std::array::from_fn(|_| rng.f32());
            let i = decide(&brain, &inputs, &mut [0.0; HIDDEN]);
            assert!((0.0..=1.0).contains(&i.breed));
            assert!((0.0..=1.0).contains(&i.rest));
            assert!((-1.0..=1.0).contains(&i.east_west));
            assert!((-1.0..=1.0).contains(&i.north_south));
        }
    }
}
