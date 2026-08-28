//! what a set of genes costs and earns. pure functions of `Genes` plus the
//! local environment - no state, no rng, no world mutation.

use ecosym_genetics::Genes;

// ponytail: these numbers are the tuning knobs of the whole model.
// as shipped, selection drives metabolism down toward its 0.2 clamp: in a
// grazed-down world the tile is empty before intake binds, so metabolism is
// mostly cost. that is a real result, not a crash, but if you want metabolism
// to settle in the interior, raise REGROWTH in ecosym-world or make intake
// matter more than upkeep here.

/// energy burned per tick just by existing
pub fn basal_cost(g: &Genes) -> f32 {
    0.05 * g.metabolism * g.size
}

/// energy burned per tick carrying this much speed around at full effort.
/// quadratic, so speed is the trait that gets expensive fastest.
pub fn movement_cost(g: &Genes) -> f32 {
    0.05 * g.metabolism * 0.4 * g.speed * g.speed
}

/// total metabolic cost of one tick spent at `effort`, the 0..1 fraction of
/// this phenotype's full stride it actually spent.
///
/// basal cost is unavoidable; the movement half is not. that is what makes
/// resting an economic choice a policy can be selected for rather than a
/// behaviour the model hands out for free.
pub fn upkeep(g: &Genes, effort: f32) -> f32 {
    basal_cost(g) + movement_cost(g) * effort.clamp(0.0, 1.0)
}

/// how much of a tile's resource this organism can take per tick
pub fn intake(g: &Genes) -> f32 {
    0.9 * g.size * g.metabolism
}

/// ticks lived before old age
pub fn lifespan(g: &Genes) -> u32 {
    (60.0 * g.size / g.metabolism) as u32
}

/// energy needed before it can breed
pub fn reproduction_threshold(g: &Genes) -> f32 {
    8.0 * g.size
}

/// how far one movement step reaches
pub fn step_length(g: &Genes) -> f32 {
    g.speed
}

/// 0.1 .. 1.0 - the fraction of food kept at this temperature
pub fn climate_fit(g: &Genes, temperature: f32) -> f32 {
    (1.0 - 1.5 * (temperature - g.heat_pref).abs()).max(0.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genes(speed: f32, metabolism: f32) -> Genes {
        Genes { speed, size: 1.0, metabolism, heat_pref: 0.5 }
    }

    #[test]
    fn upkeep_is_basal_plus_movement_and_speed_is_the_expensive_trait() {
        let g = genes(1.5, 1.0);
        assert!((upkeep(&g, 1.0) - (basal_cost(&g) + movement_cost(&g))).abs() < 1e-6);
        assert!(movement_cost(&genes(2.0, 1.0)) > 3.9 * movement_cost(&genes(1.0, 1.0)));
    }

    #[test]
    fn standing_still_costs_only_the_basal_half() {
        let g = genes(2.0, 1.0);
        assert!((upkeep(&g, 0.0) - basal_cost(&g)).abs() < 1e-6);
        assert!(upkeep(&g, 0.0) < upkeep(&g, 0.5));
        assert!(upkeep(&g, 0.5) < upkeep(&g, 1.0));
        // effort is a fraction, whatever a caller hands in
        assert_eq!(upkeep(&g, 5.0), upkeep(&g, 1.0));
        assert_eq!(upkeep(&g, -1.0), upkeep(&g, 0.0));
    }

    #[test]
    fn climate_fit_peaks_at_the_preferred_temperature_and_floors_at_a_tenth() {
        let g = genes(1.0, 1.0);
        assert!((climate_fit(&g, 0.5) - 1.0).abs() < 1e-6);
        assert!(climate_fit(&g, 0.0) < climate_fit(&g, 0.3));
        // far enough off-preference and only the floor is left
        let cold = Genes { heat_pref: 0.1, ..genes(1.0, 1.0) };
        assert_eq!(climate_fit(&cold, 1.0), 0.1);
    }

    #[test]
    fn a_fast_metabolism_shortens_life_and_raises_intake() {
        assert!(lifespan(&genes(1.0, 2.0)) < lifespan(&genes(1.0, 0.5)));
        assert!(intake(&genes(1.0, 2.0)) > intake(&genes(1.0, 0.5)));
    }
}
