//! `radar-geo` — radar polar-geometry math for RadarPro.
//!
//! This crate maps between a radar's native polar geometry (azimuth,
//! slant range, elevation angle — as carried by
//! [`radar_types::Sweep`]/[`radar_types::Radial`]/[`radar_types::Moment`])
//! and geographic latitude/longitude, plus the reverse: locating which
//! radial and gate a geographic cursor position falls on.
//!
//! # Design constraint (from `GLOBAL_CONTRACT.md`/S02)
//!
//! Polar geometry is preserved as the source of truth. This crate
//! computes geometry **on demand** from polar metadata (a `Sweep`'s
//! radials, a `Moment`'s gate spacing, and so on); it never eagerly
//! converts a whole volume or sweep into persistent latitude/longitude
//! polygons or point clouds. [`range_ring`]/[`range_rings`] are the one
//! exception in spirit but not in rule: they are rendering/UI helpers
//! that generate ring geometry from a site position on demand, entirely
//! independent of any decoded volume — they never touch or cache
//! decoded radar data.
//!
//! # Units
//!
//! - Latitude, longitude, azimuth, elevation, and bearing are always in
//!   **decimal degrees**.
//! - Range, distance, and height are always in **kilometers**, matching
//!   [`radar_types::Moment`]'s `first_gate_range_km`/`gate_spacing_km`.
//!
//! # Earth model
//!
//! See the [`earth`] module docs and
//! `docs/adr/0006-earth-model-for-radar-geometry.md` for the spherical
//! Earth + 4/3 effective-Earth-radius model used throughout this crate,
//! and why it was chosen over WGS84 ellipsoidal geodesy for this stage.
//!
//! # Module map
//!
//! - [`earth`]: Earth-model constants (mean radius, effective radius).
//! - [`spherical`]: great-circle distance/bearing, destination point,
//!   range rings.
//! - [`beam`]: beam-height and ground-range/slant-range approximations.
//! - [`lookup`]: azimuth -> radial and slant-range -> gate lookup, and
//!   the composed cursor-lat/lon -> radial/gate resolution.

pub mod beam;
pub mod earth;
pub mod lookup;
pub mod spherical;

pub use beam::{beam_height_km, ground_range_km, slant_range_from_ground_range_km};
pub use lookup::{
    cursor_to_polar, find_gate_index, find_radial_index, find_radial_index_for_sweep,
    locate_gate_value, locate_radial_gate, PolarCoord, RadialGate,
};
pub use spherical::{
    destination_point, distance_bearing, range_ring, range_rings, GreatCircle, LatLon,
};
