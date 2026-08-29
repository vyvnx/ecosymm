//! protocol v1 binary framing.
//!
//! no rust struct reaches the wire byte-for-byte: every field is written
//! explicitly and little-endian, so compiler padding can never leak into the
//! format and a browser can map the payload with typed arrays.
//!
//! ```text
//! offset 0  [u8; 4]  magic "ECSY"
//!        4  u16      protocol version
//!        6  u8       kind: 1 world, 2 snapshot
//!        7  u8       flags, 0 in v1
//!        8  u32      payload bytes after this 12-byte header
//! ```

use ecosym_simulation::{RenderSnapshot, RenderWorld, HABITABLE_FERTILITY};
use std::fmt;

pub const MAGIC: [u8; 4] = *b"ECSY";
pub const VERSION: u16 = 1;
pub const KIND_WORLD: u8 = 1;
pub const KIND_SNAPSHOT: u8 = 2;
pub const HEADER: usize = 12;

/// the browser refuses anything bigger, so refuse to build one
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// the u64 ids are read as a `BigUint64Array` view straight off the received
/// buffer, which the browser only allows on an 8-byte boundary.
const ID_ALIGNMENT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// a frame past `MAX_FRAME`, or a count past what its `u32` field holds
    TooLarge(&'static str, u64),
    /// fields that disagree about how many cells or organisms there are
    Mismatch(&'static str),
    /// a nan or infinity that would become a garbage pixel
    NonFinite,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::TooLarge(what, n) => write!(f, "{what} is too large for the wire: {n}"),
            WireError::Mismatch(what) => write!(f, "{what} disagrees with the declared count"),
            WireError::NonFinite => write!(f, "a render value is not finite"),
        }
    }
}

impl std::error::Error for WireError {}

pub fn encode_world(world: &RenderWorld) -> Result<Vec<u8>, WireError> {
    let cells = cells(world)?;
    if world.fertility.len() != cells || world.temperature.len() != cells {
        return Err(WireError::Mismatch("world field length"));
    }

    let mut buf = frame(KIND_WORLD, HEADER + 12 + cells * 2);
    buf.extend_from_slice(&world.width.to_le_bytes());
    buf.extend_from_slice(&world.height.to_le_bytes());
    buf.extend_from_slice(&(cells as u32).to_le_bytes());
    buf.extend(world.fertility.iter().map(|v| quantise(*v)));
    buf.extend(world.temperature.iter().map(|v| quantise(*v)));
    finish(buf)
}

/// the snapshot needs the world because fullness is standing over capacity,
/// and capacity is the static fertility field.
pub fn encode_snapshot(
    snapshot: &RenderSnapshot,
    world: &RenderWorld,
) -> Result<Vec<u8>, WireError> {
    let cells = cells(world)?;
    if world.fertility.len() != cells || snapshot.resources.len() != cells {
        return Err(WireError::Mismatch("resource field length"));
    }
    let count = u32::try_from(snapshot.organisms.len())
        .map_err(|_| WireError::TooLarge("organism count", snapshot.organisms.len() as u64))?;

    let size = HEADER + 16 + cells + ID_ALIGNMENT + count as usize * 24;
    let mut buf = frame(KIND_SNAPSHOT, size);
    buf.extend_from_slice(&snapshot.epoch.to_le_bytes());
    buf.extend_from_slice(&(cells as u32).to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
    buf.extend(
        snapshot.resources.iter().zip(&world.fertility).map(|(r, c)| quantise(fullness(*r, *c))),
    );
    while !buf.len().is_multiple_of(ID_ALIGNMENT) {
        buf.push(0);
    }

    // one field at a time, so the browser gets five contiguous typed arrays
    // instead of a struct it would have to walk element by element
    for o in &snapshot.organisms {
        buf.extend_from_slice(&o.id.to_le_bytes());
    }
    for o in &snapshot.organisms {
        buf.extend_from_slice(&o.species_id.to_le_bytes());
    }
    for o in &snapshot.organisms {
        buf.extend_from_slice(&display(o.x, world.width)?.to_le_bytes());
    }
    for o in &snapshot.organisms {
        buf.extend_from_slice(&display(o.y, world.height)?.to_le_bytes());
    }
    for o in &snapshot.organisms {
        if !o.energy.is_finite() {
            return Err(WireError::NonFinite);
        }
        buf.extend_from_slice(&o.energy.to_le_bytes());
    }
    finish(buf)
}

fn cells(world: &RenderWorld) -> Result<usize, WireError> {
    let cells = u64::from(world.width) * u64::from(world.height);
    u32::try_from(cells).map(|c| c as usize).map_err(|_| WireError::TooLarge("cell count", cells))
}

fn frame(kind: u8, capacity: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(capacity);
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.push(kind);
    buf.push(0); // flags
    buf.extend_from_slice(&0u32.to_le_bytes()); // payload length, patched in `finish`
    buf
}

fn finish(mut buf: Vec<u8>) -> Result<Vec<u8>, WireError> {
    if buf.len() > MAX_FRAME {
        return Err(WireError::TooLarge("frame", buf.len() as u64));
    }
    let payload = buf.len() - HEADER;
    let len = u32::try_from(payload).map_err(|_| WireError::TooLarge("payload", payload as u64))?;
    buf[8..HEADER].copy_from_slice(&len.to_le_bytes());
    Ok(buf)
}

/// visual fields are eight bits. a nan clamps to nothing and encodes as 0.
fn quantise(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// how much of this tile is left, as a viewer means it. the sea and any tile
/// too barren to grow read as empty rather than as full, so depleted land
/// stays distinguishable from water downstream.
fn fullness(standing: f32, capacity: f32) -> f32 {
    if capacity > HABITABLE_FERTILITY && standing.is_finite() {
        standing / capacity
    } else {
        0.0
    }
}

/// the geometrically expected toroidal position, for display only.
///
/// simulation state keeps the raw coordinate. `World::idx` truncates *before*
/// it wraps, so for a negative fraction the tile an organism eats from and the
/// pixel it is drawn on are one tile apart. that is the current simulation
/// behaviour, not a rounding accident, and changing it would move the digest.
fn display(v: f32, extent: u32) -> Result<f32, WireError> {
    if !v.is_finite() {
        return Err(WireError::NonFinite);
    }
    Ok(v.rem_euclid(extent.max(1) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecosym_simulation::RenderOrganism;

    /// the same literals live in `apps/web/src/render/protocol.test.js`. if a
    /// field order, an endianness, a quantisation step or the id alignment
    /// moves, both sides fail at once instead of one side drifting silently.
    const GOLDEN_WORLD: &str = concat!(
        "45435359", "0100", "01", "00", "14000000", // header: ECSY v1 world, 20 byte payload
        "02000000", "02000000", "04000000", // 2 x 2, 4 cells
        "0080ff40", // fertility 0, 0.5, 1, 0.25
        "ffbf8000", // temperature 1, 0.75, 0.5, 0
    );

    const GOLDEN_SNAPSHOT: &str = concat!(
        "45435359",
        "0100",
        "02",
        "00",
        "44000000", // header: ECSY v1 snapshot, 68 bytes
        "07000000",
        "04000000",
        "02000000",
        "00000000", // epoch 7, 4 cells, 2 organisms
        "0080ff80", // fullness: sea reads empty, then 0.5, 1, 0.5
        // 28 + 4 cells lands on 32, so this frame needs no alignment padding
        "0500000002000000",
        "0900000000000000", // ids 2^33 + 5 and 9
        "00000000",
        "01000000", // species 0 and 1
        "0000c03f",
        "0000803e", // x: -0.5 and 2.25 wrapped into a width of 2
        "0000c03f",
        "00000000", // y
        "0000a040",
        "0000803e", // energy 5 and 0.25
    );

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn world() -> RenderWorld {
        RenderWorld {
            width: 2,
            height: 2,
            fertility: vec![0.0, 0.5, 1.0, 0.25],
            temperature: vec![1.0, 0.75, 0.5, 0.0],
        }
    }

    fn snapshot() -> RenderSnapshot {
        RenderSnapshot {
            epoch: 7,
            resources: vec![0.0, 0.25, 1.0, 0.125],
            organisms: vec![
                RenderOrganism { id: (1 << 33) + 5, species_id: 0, x: -0.5, y: 1.5, energy: 5.0 },
                RenderOrganism { id: 9, species_id: 1, x: 2.25, y: 0.0, energy: 0.25 },
            ],
        }
    }

    #[test]
    fn the_world_matches_the_golden_vector() {
        assert_eq!(hex(&encode_world(&world()).unwrap()), GOLDEN_WORLD);
    }

    #[test]
    fn the_snapshot_matches_the_golden_vector() {
        assert_eq!(hex(&encode_snapshot(&snapshot(), &world()).unwrap()), GOLDEN_SNAPSHOT);
    }

    #[test]
    fn the_declared_payload_length_is_the_bytes_that_follow_the_header() {
        for bytes in
            [encode_world(&world()).unwrap(), encode_snapshot(&snapshot(), &world()).unwrap()]
        {
            let declared = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
            assert_eq!(declared + HEADER, bytes.len());
            assert_eq!(&bytes[0..4], &MAGIC);
            assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), VERSION);
            assert_eq!(bytes[7], 0, "v1 declares no flags");
        }
    }

    #[test]
    fn quantisation_is_exact_at_the_ends_and_clamps_outside_them() {
        assert_eq!(quantise(0.0), 0);
        assert_eq!(quantise(0.5), 128);
        assert_eq!(quantise(1.0), 255);
        assert_eq!(quantise(-3.0), 0);
        assert_eq!(quantise(7.0), 255);
        assert_eq!(quantise(f32::NAN), 0);
    }

    #[test]
    fn fullness_reads_empty_wherever_nothing_can_grow() {
        assert_eq!(fullness(5.0, 0.0), 0.0);
        assert_eq!(fullness(5.0, HABITABLE_FERTILITY), 0.0);
        assert_eq!(fullness(5.0, -1.0), 0.0);
        assert_eq!(fullness(f32::NAN, 1.0), 0.0);
        assert_eq!(fullness(0.5, 1.0), 0.5);
        // a full tile is full whatever its capacity is
        assert_eq!(fullness(0.2, 0.2), 1.0);
    }

    /// the seam regression: a negative crossing normalises the geometric way,
    /// and the exact boundary lands on zero rather than on the far edge
    #[test]
    fn positions_normalise_onto_the_torus_from_both_sides() {
        assert_eq!(display(-0.5, 128).unwrap(), 127.5);
        assert_eq!(display(-128.25, 128).unwrap(), 127.75);
        assert_eq!(display(128.0, 128).unwrap(), 0.0);
        assert_eq!(display(129.5, 128).unwrap(), 1.5);
        assert_eq!(display(0.0, 128).unwrap(), 0.0);
        assert_eq!(display(0.5, 1).unwrap(), 0.5);
        assert_eq!(display(f32::INFINITY, 128), Err(WireError::NonFinite));

        // a negative fraction is exactly where this rule and `World::idx`
        // part company: idx truncates to tile 0, this lands on tile 127.
        // `ecosym_world`'s own `a_negative_fraction_truncates_before_it_wraps`
        // holds the other half of the pair.
        assert_eq!(display(-0.5, 128).unwrap() as usize, 127);
    }

    #[test]
    fn an_empty_snapshot_is_a_valid_frame() {
        let empty = RenderSnapshot { epoch: 0, resources: vec![0.0; 4], organisms: vec![] };
        let bytes = encode_snapshot(&empty, &world()).unwrap();
        assert_eq!(bytes.len(), HEADER + 16 + 4);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 0);
    }

    /// 28 + cells has to reach an 8-byte boundary before the ids start, and
    /// the bytes that get it there must be zero
    #[test]
    fn the_ids_start_on_an_eight_byte_boundary_with_zero_padding() {
        for cells in 1..40usize {
            let world = RenderWorld {
                width: cells as u32,
                height: 1,
                fertility: vec![1.0; cells],
                temperature: vec![0.5; cells],
            };
            let snap = RenderSnapshot {
                epoch: 1,
                resources: vec![1.0; cells],
                organisms: vec![RenderOrganism {
                    id: 1,
                    species_id: 0,
                    x: 0.0,
                    y: 0.0,
                    energy: 1.0,
                }],
            };
            let bytes = encode_snapshot(&snap, &world).unwrap();
            let ids_at = bytes.len() - 24;
            assert!(ids_at.is_multiple_of(ID_ALIGNMENT), "{cells} cells misaligned the id array");
            assert!(bytes[HEADER + 16 + cells..ids_at].iter().all(|b| *b == 0));
        }
    }

    #[test]
    fn disagreeing_field_lengths_are_refused() {
        let mut broken = world();
        broken.fertility.pop();
        assert_eq!(encode_world(&broken), Err(WireError::Mismatch("world field length")));

        let short = RenderSnapshot { epoch: 0, resources: vec![0.0; 3], organisms: vec![] };
        assert_eq!(
            encode_snapshot(&short, &world()),
            Err(WireError::Mismatch("resource field length"))
        );
    }

    #[test]
    fn a_cell_count_that_overflows_a_u32_is_refused() {
        let huge =
            RenderWorld { width: 70_000, height: 70_000, fertility: vec![], temperature: vec![] };
        assert_eq!(encode_world(&huge), Err(WireError::TooLarge("cell count", 4_900_000_000)));
    }

    #[test]
    fn a_non_finite_organism_never_reaches_the_wire() {
        for broken in [f32::NAN, f32::INFINITY] {
            let mut snap = snapshot();
            snap.organisms[0].x = broken;
            assert_eq!(encode_snapshot(&snap, &world()), Err(WireError::NonFinite));

            let mut snap = snapshot();
            snap.organisms[1].energy = broken;
            assert_eq!(encode_snapshot(&snap, &world()), Err(WireError::NonFinite));
        }
    }
}
