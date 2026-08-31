/**
 * what a species is, in words a bettor can read.
 *
 * the market card is the one place anyone decides between two species, and
 * "speed 1.3" is not a fact anyone can bet on. every band below is measured
 * against the neutral body the ecology is balanced around - 1.0 on each body
 * scale, 0.5 on heat - so a word means the same thing in every market.
 *
 * nothing here is a prediction, a fitness reading or a hint about the pools.
 * it is the founder body the run will start from and nothing else: what
 * selection then does to it is the run's own story.
 */

/** trait, its neutral point, the deadband around it, and the word either side */
const BANDS = [
  ['speed', 1.0, 0.15, 'fast', 'slow'],
  ['size', 1.0, 0.15, 'large', 'small'],
  ['metabolism', 1.0, 0.15, 'hungry', 'thrifty'],
  // a narrower band because the whole scale is 0..1: 0.44 to 0.56 is the
  // middle latitudes, and a species there has no preference worth printing
  ['heat_pref', 0.5, 0.06, 'likes warmth', 'likes cold'],
]

/**
 * a founder body as a handful of words. a trait sitting inside its deadband
 * is left out rather than named: what is worth printing is where a species
 * differs from an ordinary body, not that it has one.
 */
export function traits(genes) {
  if (!genes) return []
  const words = BANDS.flatMap(([key, pivot, band, high, low]) => {
    const value = genes[key]
    if (!Number.isFinite(value)) return []
    if (value > pivot + band) return [high]
    if (value < pivot - band) return [low]
    return []
  })
  return words.length > 0 ? words : ['unremarkable']
}

/**
 * founder to final, for the settled card. one line per species, and it says
 * what moved rather than who deserved to win.
 */
export function evolutionSummary(result) {
  const moved = [
    ['speed', result.founder_genes.speed, result.final_genes.speed],
    ['metabolism', result.founder_genes.metabolism, result.final_genes.metabolism],
    ['heat_pref', result.founder_genes.heat_pref, result.final_genes.heat_pref],
  ]
    .map(([label, from, to]) => ({ label, from, to, moved: Math.abs(to - from) }))
    .sort((a, b) => b.moved - a.moved)
  return moved.slice(0, 2)
}
