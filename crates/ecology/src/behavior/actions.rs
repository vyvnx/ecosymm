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
/// headings; the other two are 0..1 pressures.
///
/// there is no food-seeking channel any more. the resource direction is an
/// observation the policy weights like any other, so a heading toward food is
/// something a brain has to learn to produce rather than a lever the ecology
/// layer wired to an answer it worked out on the organism's behalf.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    pub east_west: f32,
    pub north_south: f32,
    pub breed: f32,
    pub rest: f32,
}

impl Intent {
    /// decode one forward pass. outputs arrive in -1..1 from `tanh`; the two
    /// pressures are rescaled to 0..1 and the headings are left signed.
    pub fn decode(outputs: [f32; OUTPUTS]) -> Intent {
        Intent {
            east_west: outputs[0],
            north_south: outputs[1],
            breed: unit(outputs[2]),
            rest: unit(outputs[3]),
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

/// take the policy's heading, add the tick's wander, then cap the result at
/// what this body can do.
///
/// nothing here knows where the food is. the policy's heading is the whole of
/// where an organism wants to go, and it is free to evolve into tracking a
/// resource, avoiding a crowd, holding a bearing or standing still.
///
/// `wander` is part of the *intended* displacement, not a free offset bolted on
/// afterwards. it therefore lands inside the stride cap, is suppressed by rest
/// along with everything else, and is paid for as movement effort - a resting
/// organism is genuinely still, and nobody drifts for nothing.
pub fn stride(g: &Genes, intent: &Intent, wander: (f32, f32)) -> Stride {
    let heading = (intent.east_west + wander.0, intent.north_south + wander.1);
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

/// what one organism actually did in one tick, and where it did it. purely
/// descriptive: these are folded into per-species statistics and nothing ever
/// reads them back into the simulation.
///
/// `tracking` is measured from the displacement rather than read off an output,
/// which is why it survived the removal of the food-seeking channel: it says
/// what happened instead of what was wanted.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Act {
    pub moved: f32,
    pub tracking: f32,
    pub reproduction: f32,
    pub resting: f32,
    pub competitors: f32,
    /// the temperature of the tile it ended the tick on, and how well its
    /// genes suit that temperature. the realised climate niche, as opposed to
    /// the `heat_pref` it inherited.
    pub temperature: f32,
    pub climate_fit: f32,
}

impl Act {
    /// the same seven numbers in the fixed order every tally and report uses.
    /// one list, so a field added here cannot be forgotten in three places.
    pub fn channels(&self) -> [f32; CHANNELS] {
        [
            self.moved,
            self.tracking,
            self.reproduction,
            self.resting,
            self.competitors,
            self.temperature,
            self.climate_fit,
        ]
    }
}

/// how many descriptive channels one act carries
pub const CHANNELS: usize = 7;

/// running behavioural totals for one species over one epoch.
///
/// count, sum and sum of squares per channel - enough for the mean and the
/// variance in one pass, with no history retained. `f64` because these sum over
/// millions of organism-ticks; the reported figures come back out as `f32`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BehaviorTally {
    acts: u64,
    sum: [f64; CHANNELS],
    sum_squares: [f64; CHANNELS],
}

impl Default for BehaviorTally {
    fn default() -> BehaviorTally {
        BehaviorTally { acts: 0, sum: [0.0; CHANNELS], sum_squares: [0.0; CHANNELS] }
    }
}

impl BehaviorTally {
    pub fn record(&mut self, act: &Act) {
        self.acts += 1;
        for (i, v) in act.channels().iter().enumerate() {
            self.sum[i] += *v as f64;
            self.sum_squares[i] += (*v as f64) * (*v as f64);
        }
    }

    pub fn acts(&self) -> u64 {
        self.acts
    }

    pub fn mean(&self) -> BehaviorStats {
        if self.acts == 0 {
            return BehaviorStats::default();
        }
        let n = self.acts as f64;
        BehaviorStats::from_channels(std::array::from_fn(|i| (self.sum[i] / n) as f32))
    }

    /// spread across organism-ticks, per channel.
    ///
    /// **this is organism-tick action variance and nothing more.** it does not
    /// establish that individuals hold persistently different strategies: one
    /// organism behaving differently at different moments produces exactly the
    /// same number as two organisms behaving differently from each other.
    pub fn variance(&self) -> BehaviorStats {
        if self.acts == 0 {
            return BehaviorStats::default();
        }
        let n = self.acts as f64;
        BehaviorStats::from_channels(std::array::from_fn(|i| {
            let mean = self.sum[i] / n;
            ((self.sum_squares[i] / n) - mean * mean).max(0.0) as f32
        }))
    }
}

/// one species' behavioural fingerprint over an epoch, as either means or
/// variances across every organism-tick.
///
/// **descriptive only.** nothing selects on these, nothing feeds them back, and
/// there is no fitness function hiding in here. the environment is the fitness
/// function; this is the instrument reading it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BehaviorStats {
    /// distance actually travelled per tick
    pub movement: f32,
    /// how well the movement lined up with the resource direction the organism
    /// was shown, -1..1. measured from the displacement, not read off a policy
    /// output.
    pub resource_tracking: f32,
    pub reproduction: f32,
    pub resting: f32,
    /// mean local competing-species density the species was exposed to
    pub competitor_exposure: f32,
    /// the realised climate niche: the temperature the species actually
    /// occupied, and how well its genes suited it. `heat_pref` is what it
    /// inherited; this is where it ended up living.
    pub occupied_temperature: f32,
    pub climate_fit: f32,
}

impl BehaviorStats {
    /// the same seven numbers in the same fixed order `Act::channels` uses.
    /// the replay digest folds this rather than seven named fields, so a
    /// channel added to the model cannot quietly stay out of the digest.
    pub fn channels(&self) -> [f32; CHANNELS] {
        [
            self.movement,
            self.resource_tracking,
            self.reproduction,
            self.resting,
            self.competitor_exposure,
            self.occupied_temperature,
            self.climate_fit,
        ]
    }

    fn from_channels(c: [f32; CHANNELS]) -> BehaviorStats {
        BehaviorStats {
            movement: c[0],
            resource_tracking: c[1],
            reproduction: c[2],
            resting: c[3],
            competitor_exposure: c[4],
            occupied_temperature: c[5],
            climate_fit: c[6],
        }
    }

    pub fn is_finite(&self) -> bool {
        self.channels().iter().all(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genes(speed: f32) -> Genes {
        Genes { speed, size: 1.0, metabolism: 1.0, heat_pref: 0.5 }
    }

    fn intent(ew: f32, ns: f32, rest: f32) -> Intent {
        Intent { east_west: ew, north_south: ns, breed: 0.5, rest }
    }

    #[test]
    fn decoding_leaves_headings_signed_and_lifts_pressures_into_zero_one() {
        let i = Intent::decode([-1.0, 1.0, 0.0, 1.0]);
        assert_eq!((i.east_west, i.north_south), (-1.0, 1.0));
        assert_eq!((i.breed, i.rest), (0.5, 1.0));
    }

    #[test]
    fn a_stride_never_exceeds_one_step_whatever_the_policy_asks() {
        let g = genes(2.0);
        for (ew, ns) in [(1.0, 1.0), (-1.0, 1.0), (1.0, 0.0), (0.3, -0.9)] {
            let s = stride(&g, &intent(ew, ns, 0.0), (0.3, -0.3));
            let travelled = (s.dx * s.dx + s.dy * s.dy).sqrt();
            assert!(travelled <= phenotype::step_length(&g) + 1e-5, "{travelled}");
            assert!((0.0..=1.0).contains(&s.effort));
        }
    }

    /// rest suppresses the whole intended displacement, wander included
    #[test]
    fn resting_costs_nothing_and_goes_nowhere() {
        let s = stride(&genes(2.0), &intent(1.0, 1.0, 1.0), (0.4, -0.2));
        assert_eq!(s.effort, 0.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0));
    }

    #[test]
    fn a_policy_with_no_heading_and_no_wander_stands_still() {
        assert_eq!(stride(&genes(1.0), &intent(0.0, 0.0, 0.0), (0.0, 0.0)), Stride::default());
    }

    /// the food gradient is gone from movement entirely: the heading is the
    /// only thing that decides where a body goes
    #[test]
    fn the_heading_is_the_only_thing_that_moves_a_body() {
        let g = genes(1.0);
        let west = stride(&g, &intent(-1.0, 0.0, 0.0), (0.0, 0.0));
        let east = stride(&g, &intent(1.0, 0.0, 0.0), (0.0, 0.0));
        assert!(west.dx < 0.0 && east.dx > 0.0);
        assert!((west.dx + east.dx).abs() < 1e-6);
    }

    fn act(moved: f32, tracking: f32) -> Act {
        Act { moved, tracking, ..Act::default() }
    }

    #[test]
    fn a_tally_averages_over_organism_ticks_and_an_empty_one_is_zero() {
        let mut tally = BehaviorTally::default();
        assert_eq!(tally.mean(), BehaviorStats::default());
        assert_eq!(tally.variance(), BehaviorStats::default());

        tally.record(&Act { moved: 1.0, resting: 1.0, ..act(1.0, 0.0) });
        tally.record(&Act { moved: 0.0, resting: 0.0, ..act(0.0, 1.0) });
        let mean = tally.mean();
        assert_eq!(tally.acts(), 2);
        assert_eq!((mean.movement, mean.resource_tracking, mean.resting), (0.5, 0.5, 0.5));
        assert!(mean.is_finite());
    }

    /// the variance is what a "strategy shifting" reading is built on, so it
    /// has to be zero when nothing varies and positive when something does
    #[test]
    fn variance_is_zero_for_identical_acts_and_positive_otherwise() {
        let mut same = BehaviorTally::default();
        for _ in 0..100 {
            same.record(&act(0.7, -0.2));
        }
        assert!(same.variance().movement.abs() < 1e-6);
        assert!(same.variance().resource_tracking.abs() < 1e-6);

        let mut split = BehaviorTally::default();
        for i in 0..100 {
            split.record(&act(if i % 2 == 0 { 0.0 } else { 1.0 }, 0.0));
        }
        // a fair two-point split at 0 and 1 has variance 0.25
        assert!((split.variance().movement - 0.25).abs() < 1e-5, "{}", split.variance().movement);
        assert!(split.variance().is_finite());
    }

    #[test]
    fn the_realised_niche_is_tallied_alongside_the_actions() {
        let mut tally = BehaviorTally::default();
        tally.record(&Act { temperature: 0.2, climate_fit: 0.4, ..Act::default() });
        tally.record(&Act { temperature: 0.6, climate_fit: 0.8, ..Act::default() });
        let mean = tally.mean();
        assert!((mean.occupied_temperature - 0.4).abs() < 1e-6);
        assert!((mean.climate_fit - 0.6).abs() < 1e-6);
    }
}
