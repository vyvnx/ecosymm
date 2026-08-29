//! the inherited brain: one fixed-topology feed-forward network, stored as flat
//! numeric arrays.
//!
//! genetics owns the numbers and the forward pass that gives them meaning. what
//! an organism *does* with the outputs is `ecosym-ecology::behavior`, the same
//! way `Genes` live here and what they cost lives in `ecology::phenotype`.

use ecosym_core::Rng;

/// 12 -> 8 -> 4, identical for every organism in the run. only the numbers
/// evolve; evolving the topology itself (NEAT and friends) is out of scope.
///
/// the input count is the observation contract in `ecology::observations` and
/// the output count is the decode in `ecology::actions`. all three move
/// together or the network is reading one thing and answering another.
pub const INPUTS: usize = 12;
pub const HIDDEN: usize = 8;
pub const OUTPUTS: usize = 4;

pub const WEIGHT_COUNT: usize = INPUTS * HIDDEN + HIDDEN * OUTPUTS;
pub const BIAS_COUNT: usize = HIDDEN + OUTPUTS;

/// weights clamp here, so a long mutation chain cannot walk off to infinity and
/// take the forward pass with it
pub const WEIGHT_BOUNDS: (f32, f32) = (-4.0, 4.0);

/// half-width of a founder weight.
///
/// the textbook `tanh` initialisation is `1/sqrt(fan_in)`, about 0.35 here, and
/// it was tried first. it is wrong for this model: it makes every founder
/// behave alike and near the middle of every tendency, which in this economy is
/// a policy that forages too weakly to reach the breeding threshold before old
/// age - whole scenarios go extinct with nothing selected. the full range gives
/// founders opinions. many of those opinions are lethal, and that first
/// die-off *is* the selection event.
pub const FOUNDER_WEIGHT: f32 = 1.0;

/// a brain. two contiguous `f32` arrays and nothing else: no map, no graph, no
/// boxed neuron, so a device buffer is a memcpy away when one is wanted.
///
/// `weights` is input->hidden first, `HIDDEN` rows of `INPUTS`, then
/// hidden->output, `OUTPUTS` rows of `HIDDEN`. `biases` is the hidden layer
/// then the output layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NeuralGenome {
    pub weights: [f32; WEIGHT_COUNT],
    pub biases: [f32; BIAS_COUNT],
}

impl Default for NeuralGenome {
    fn default() -> NeuralGenome {
        NeuralGenome { weights: [0.0; WEIGHT_COUNT], biases: [0.0; BIAS_COUNT] }
    }
}

impl NeuralGenome {
    /// a brain drawn from nothing but a deterministic stream. no profile is
    /// hand-written, so no species can be handed a better policy to start with.
    pub fn random(rng: &mut Rng) -> NeuralGenome {
        let mut brain = NeuralGenome::default();
        for w in brain.weights.iter_mut().chain(brain.biases.iter_mut()) {
            *w = rng.between(-FOUNDER_WEIGHT, FOUNDER_WEIGHT);
        }
        brain
    }

    /// one forward pass. `tanh` on both layers, so every output is in -1..1 and
    /// the caller never has to defend against an unbounded tendency.
    ///
    /// ponytail: `tanh` is 12 libm calls per organism-tick and about half the
    /// wall clock of a default run (21.6s against 10.0s measured with the
    /// softsign `x / (1 + |x|)`, which evolves just as well and is bit-identical
    /// across platforms). kept because it is the standard shape and the run is
    /// fast enough; swap it here, in one line, if the tick loop ever becomes
    /// the thing worth optimising - and re-record `benchmarks/` when you do.
    pub fn forward(&self, inputs: &[f32; INPUTS]) -> [f32; OUTPUTS] {
        let mut hidden = [0.0f32; HIDDEN];
        for (h, neuron) in hidden.iter_mut().enumerate() {
            let row = &self.weights[h * INPUTS..(h + 1) * INPUTS];
            let mut sum = self.biases[h];
            for (w, x) in row.iter().zip(inputs) {
                sum += w * x;
            }
            *neuron = sum.tanh();
        }

        let second = INPUTS * HIDDEN;
        let mut outputs = [0.0f32; OUTPUTS];
        for (o, neuron) in outputs.iter_mut().enumerate() {
            let row = &self.weights[second + o * HIDDEN..second + (o + 1) * HIDDEN];
            let mut sum = self.biases[HIDDEN + o];
            for (w, h) in row.iter().zip(&hidden) {
                sum += w * h;
            }
            *neuron = sum.tanh();
        }
        outputs
    }

    /// the per-gene mean of a set of brains: where a population's policies
    /// sit, taken as one point. reported, never selected on.
    pub fn centroid<'a>(brains: impl Iterator<Item = &'a NeuralGenome>) -> NeuralGenome {
        let mut sum = NeuralGenome::default();
        let mut n = 0.0f32;
        for brain in brains {
            for (acc, w) in sum.weights.iter_mut().zip(&brain.weights) {
                *acc += w;
            }
            for (acc, b) in sum.biases.iter_mut().zip(&brain.biases) {
                *acc += b;
            }
            n += 1.0;
        }
        if n == 0.0 {
            return sum;
        }
        for w in sum.weights.iter_mut().chain(sum.biases.iter_mut()) {
            *w /= n;
        }
        sum
    }

    /// mean absolute difference per gene. how far one policy has drifted from
    /// another - reported, never selected on.
    pub fn distance(&self, other: &NeuralGenome) -> f32 {
        let sum: f32 = self.genes().zip(other.genes()).map(|(a, b)| (a - b).abs()).sum();
        sum / (WEIGHT_COUNT + BIAS_COUNT) as f32
    }

    /// mean of every weight and bias. a compact signature of the whole brain,
    /// so neural genes reach the replay digest without hashing 140 floats per
    /// organism per epoch.
    pub fn mean(&self) -> f32 {
        self.genes().sum::<f32>() / (WEIGHT_COUNT + BIAS_COUNT) as f32
    }

    /// every heritable number, weights then biases, in one stable order
    pub fn genes(&self) -> impl Iterator<Item = f32> + '_ {
        self.weights.iter().chain(self.biases.iter()).copied()
    }

    pub fn is_finite(&self) -> bool {
        self.genes().all(|w| w.is_finite())
    }

    pub fn in_bounds(&self) -> bool {
        self.genes().all(|w| (WEIGHT_BOUNDS.0..=WEIGHT_BOUNDS.1).contains(&w))
    }
}

pub(crate) fn clamp(v: f32) -> f32 {
    v.clamp(WEIGHT_BOUNDS.0, WEIGHT_BOUNDS.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(v: f32) -> [f32; INPUTS] {
        [v; INPUTS]
    }

    #[test]
    fn the_topology_is_fixed_and_the_arrays_match_it() {
        assert_eq!(WEIGHT_COUNT, 12 * 8 + 8 * 4);
        assert_eq!(BIAS_COUNT, 8 + 4);
        let brain = NeuralGenome::default();
        assert_eq!(brain.genes().count(), WEIGHT_COUNT + BIAS_COUNT);
    }

    #[test]
    fn the_forward_pass_is_deterministic_and_bounded() {
        let brain = NeuralGenome::random(&mut Rng::new(7));
        let a = brain.forward(&inputs(0.3));
        assert_eq!(a, brain.forward(&inputs(0.3)));
        assert_ne!(a, brain.forward(&inputs(0.9)));

        // saturating inputs and clamped weights still cannot leave -1..1
        let hot = NeuralGenome {
            weights: [WEIGHT_BOUNDS.1; WEIGHT_COUNT],
            biases: [WEIGHT_BOUNDS.1; BIAS_COUNT],
        };
        for out in hot.forward(&inputs(1.0)) {
            assert!((-1.0..=1.0).contains(&out), "{out}");
        }
    }

    #[test]
    fn a_zero_brain_answers_zero_and_biases_alone_move_the_output() {
        assert_eq!(NeuralGenome::default().forward(&inputs(1.0)), [0.0; OUTPUTS]);
        let mut biased = NeuralGenome::default();
        biased.biases[HIDDEN] = 1.0;
        assert!(biased.forward(&inputs(0.0))[0] > 0.5);
    }

    #[test]
    fn founders_drawn_from_one_stream_are_all_different_and_in_bounds() {
        let mut rng = Rng::new(11);
        let founders: Vec<NeuralGenome> =
            (0..200).map(|_| NeuralGenome::random(&mut rng)).collect();
        assert!(founders.iter().all(|b| b.in_bounds() && b.is_finite()));
        assert_ne!(founders[0], founders[1]);
        assert_ne!(founders[0].forward(&inputs(0.4)), founders[1].forward(&inputs(0.4)));
    }

    #[test]
    fn a_centroid_averages_a_population_and_an_empty_one_is_zero() {
        let mut rng = Rng::new(12);
        assert_eq!(NeuralGenome::centroid([].iter()), NeuralGenome::default());

        let one = NeuralGenome::random(&mut rng);
        assert_eq!(NeuralGenome::centroid([one].iter()), one);

        let mut low = NeuralGenome::default();
        let mut high = NeuralGenome::default();
        for (a, b) in low.weights.iter_mut().zip(high.weights.iter_mut()) {
            (*a, *b) = (-1.0, 3.0);
        }
        assert_eq!(NeuralGenome::centroid([low, high].iter()).weights[0], 1.0);
    }

    #[test]
    fn random_brains_are_seed_deterministic_and_seed_sensitive() {
        assert_eq!(NeuralGenome::random(&mut Rng::new(3)), NeuralGenome::random(&mut Rng::new(3)));
        assert_ne!(NeuralGenome::random(&mut Rng::new(3)), NeuralGenome::random(&mut Rng::new(4)));
    }

    #[test]
    fn distance_and_mean_summarise_the_whole_brain() {
        let a = NeuralGenome::default();
        let mut b = a;
        assert_eq!(a.distance(&b), 0.0);
        for w in b.weights.iter_mut() {
            *w = 1.0;
        }
        assert!(
            (a.distance(&b) - WEIGHT_COUNT as f32 / (WEIGHT_COUNT + BIAS_COUNT) as f32).abs()
                < 1e-6
        );
        assert!((b.mean() - WEIGHT_COUNT as f32 / (WEIGHT_COUNT + BIAS_COUNT) as f32).abs() < 1e-6);
    }
}
