//! action tendencies, and what the world lets an organism actually do with
//! them.
//!
//! the policy proposes; the numbers here dispose. nothing in this file can be
//! made to exceed a phenotype's stride or skip a metabolic bill, whatever the
//! network asks for.

use crate::phenotype;
use ecosym_genetics::{Genes, OUTPUTS};
use serde::{Deserialize, Serialize};

/// what a policy wants this tick. the two movement tendencies are signed
/// headings; the other three are 0..1 pressures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    pub east_west: f32,
    pub north_south: f32,
    pub seek: f32,
    pub breed: f32,
    pub rest: f32,
}

impl Intent {
    /// decode one forward pass. outputs arrive in -1..1 from `tanh`; the three
    /// pressures are rescaled to 0..1 and the headings are left signed.
    pub fn decode(outputs: [f32; OUTPUTS]) -> Intent {
        Intent {
            east_west: outputs[0],
            north_south: outputs[1],
            seek: unit(outputs[2]),
            breed: unit(outputs[3]),
            rest: unit(outputs[4]),
        }
    }
}

fn unit(v: f32) -> f32 {
    (v + 1.0) * 0.5
}

/// the movement ecology will allow: an offset, and the fraction of this
/// phenotype's full stride it costs.
///
/// `effort` is what the energy bill is written against, so standing still is
/// genuinely cheaper than sprinting and a policy can be selected for either.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stride {
    pub dx: f32,
    pub dy: f32,
    pub effort: f32,
}

/// blend the policy's own heading with the local food gradient in the
/// proportion it asked for, then cap the result at what this body can do.
///
/// `seek` is the only channel through which the food gradient reaches
/// movement, and the organism sets it: a policy is free to evolve to ignore
/// food entirely, wander on its own heading, or stand still.
pub fn stride(g: &Genes, intent: &Intent, gradient: (f32, f32)) -> Stride {
    let heading = (
        intent.east_west * (1.0 - intent.seek) + gradient.0 * intent.seek,
        intent.north_south * (1.0 - intent.seek) + gradient.1 * intent.seek,
    );
    let len = (heading.0 * heading.0 + heading.1 * heading.1).sqrt();
    if len <= f32::EPSILON {
        return Stride::default();
    }
    // a half-hearted heading is a half-hearted stride, and rest suppresses the
    // whole thing. both ends stay inside one full step.
    let effort = ((1.0 - intent.rest) * len.min(1.0)).clamp(0.0, 1.0);
    let reach = phenotype::step_length(g) * effort;
    Stride { dx: heading.0 / len * reach, dy: heading.1 / len * reach, effort }
}

/// what one organism actually did in one tick. purely descriptive: these are
/// folded into per-species statistics and nothing ever reads them back into the
/// simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Act {
    pub moved: f32,
    pub food_seeking: f32,
    pub reproduction: f32,
    pub resting: f32,
    pub competitors: f32,
}

/// running behavioural totals for one species over one epoch.
///
/// `f64` because these sum over millions of organism-ticks; the reported means
/// come back out as `f32`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BehaviorTally {
    acts: u64,
    moved: f64,
    food_seeking: f64,
    reproduction: f64,
    resting: f64,
    competitors: f64,
}

impl BehaviorTally {
    pub fn record(&mut self, act: &Act) {
        self.acts += 1;
        self.moved += act.moved as f64;
        self.food_seeking += act.food_seeking as f64;
        self.reproduction += act.reproduction as f64;
        self.resting += act.resting as f64;
        self.competitors += act.competitors as f64;
    }

    pub fn acts(&self) -> u64 {
        self.acts
    }

    pub fn mean(&self) -> BehaviorStats {
        if self.acts == 0 {
            return BehaviorStats::default();
        }
        let n = self.acts as f64;
        BehaviorStats {
            movement: (self.moved / n) as f32,
            food_seeking: (self.food_seeking / n) as f32,
            reproduction: (self.reproduction / n) as f32,
            resting: (self.resting / n) as f32,
            competitor_exposure: (self.competitors / n) as f32,
        }
    }
}

/// one species' behavioural fingerprint: means over every organism-tick of the
/// epoch.
///
/// **descriptive only.** nothing selects on these, nothing feeds them back, and
/// there is no fitness function hiding in here. the environment is the fitness
/// function; this is the instrument reading it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BehaviorStats {
    /// distance actually travelled per tick
    pub movement: f32,
    pub food_seeking: f32,
    pub reproduction: f32,
    pub resting: f32,
    /// mean local competing-species density the species was exposed to
    pub competitor_exposure: f32,
}

impl BehaviorStats {
    pub fn is_finite(&self) -> bool {
        self.movement.is_finite()
            && self.food_seeking.is_finite()
            && self.reproduction.is_finite()
            && self.resting.is_finite()
            && self.competitor_exposure.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genes(speed: f32) -> Genes {
        Genes { speed, size: 1.0, metabolism: 1.0, heat_pref: 0.5 }
    }

    fn intent(ew: f32, ns: f32, seek: f32, rest: f32) -> Intent {
        Intent { east_west: ew, north_south: ns, seek, breed: 0.5, rest }
    }

    #[test]
    fn decoding_leaves_headings_signed_and_lifts_pressures_into_zero_one() {
        let i = Intent::decode([-1.0, 1.0, -1.0, 0.0, 1.0]);
        assert_eq!((i.east_west, i.north_south), (-1.0, 1.0));
        assert_eq!((i.seek, i.breed, i.rest), (0.0, 0.5, 1.0));
    }

    #[test]
    fn a_stride_never_exceeds_one_step_whatever_the_policy_asks() {
        let g = genes(2.0);
        for (ew, ns) in [(1.0, 1.0), (-1.0, 1.0), (1.0, 0.0), (0.3, -0.9)] {
            let s = stride(&g, &intent(ew, ns, 0.0, 0.0), (0.0, 0.0));
            let travelled = (s.dx * s.dx + s.dy * s.dy).sqrt();
            assert!(travelled <= phenotype::step_length(&g) + 1e-5, "{travelled}");
            assert!((0.0..=1.0).contains(&s.effort));
        }
    }

    #[test]
    fn resting_costs_nothing_and_goes_nowhere() {
        let s = stride(&genes(2.0), &intent(1.0, 1.0, 0.0, 1.0), (1.0, 0.0));
        assert_eq!(s.effort, 0.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0));
    }

    #[test]
    fn a_policy_with_no_heading_and_no_seeking_stands_still() {
        assert_eq!(stride(&genes(1.0), &intent(0.0, 0.0, 0.0, 0.0), (1.0, 0.0)), Stride::default());
    }

    #[test]
    fn seeking_hands_the_heading_over_to_the_gradient() {
        let g = genes(1.0);
        // the policy wants to go west, the food is east. at seek 1 the food wins.
        let ignored = stride(&g, &intent(-1.0, 0.0, 0.0, 0.0), (1.0, 0.0));
        let followed = stride(&g, &intent(-1.0, 0.0, 1.0, 0.0), (1.0, 0.0));
        assert!(ignored.dx < 0.0, "the policy's own heading was overridden");
        assert!(followed.dx > 0.0, "seeking did not reach the gradient");
    }

    #[test]
    fn a_tally_averages_over_organism_ticks_and_an_empty_one_is_zero() {
        let mut tally = BehaviorTally::default();
        assert_eq!(tally.mean(), BehaviorStats::default());
        tally.record(&Act { moved: 1.0, food_seeking: 0.0, resting: 1.0, ..Act::default() });
        tally.record(&Act { moved: 0.0, food_seeking: 1.0, resting: 0.0, ..Act::default() });
        let mean = tally.mean();
        assert_eq!(tally.acts(), 2);
        assert_eq!((mean.movement, mean.food_seeking, mean.resting), (0.5, 0.5, 0.5));
        assert!(mean.is_finite());
    }
}
