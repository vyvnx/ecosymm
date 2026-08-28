//! gpu acceleration. empty on purpose.
//!
// ponytail: no wgpu until the cpu epoch loop is the measured bottleneck.
// benchmarks/ is where that proof goes; add the wgpu dep then, not now.
//
// when it happens: depend on ecosym-simulation, implement `EpochEngine`, and
// run `ecosym_simulation::conformance::verify_engine` against it. nothing in
// simulation, ecology, genetics or world should need to change.
