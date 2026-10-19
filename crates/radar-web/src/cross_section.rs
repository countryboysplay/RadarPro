//! Pure, host-testable vertical cross-section (RHI-style slice) through an
//! already-decoded [`Volume`]: samples one moment along a great-circle line
//! between two points, from the ground up to a ceiling height.
//!
//! Each output cell takes the gate of whichever sweep's beam *center* passes
//! closest to that cell's height, and only if the cell lies inside that
//! beam's nominal 1-degree width -- gaps between tilts stay empty rather
//! than being interpolated (GLOBAL_CONTRACT: no fabricated meteorology).

use radar_geo::{
    beam_height_km, distance_bearing, find_gate_index, find_radial_index_for_sweep,
    slant_range_from_ground_range_km, LatLon,
};
use radar_types::{GateValue, Moment, MomentKind, Sweep, Volume};

/// Nominal WSR-88D half-power beamwidth (degrees).
const BEAMWIDTH_DEG: f64 = 0.95;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrossSectionCell {
    /// No beam covers this cell, or the gate is missing.
    NoData,
    RangeFolded,
    Value(f32),
}

/// `cols * rows` cells, row-major, row 0 at the top (`top_km` MSL) and the
/// last row at 0 km MSL; column 0 at `start`, the last column at `end`.
pub fn cross_section(
    volume: &Volume,
    moment: MomentKind,
    start: LatLon,
    end: LatLon,
    cols: usize,
    rows: usize,
    top_km: f64,
) -> Vec<CrossSectionCell> {
    let site = LatLon::new(volume.site.latitude_deg, volume.site.longitude_deg);
    let site_height_km = volume.site.height_m / 1000.0;
    // Split-cut volumes repeat low tilts with different moments; keep only
    // sweeps that actually carry the requested one.
    let sweeps: Vec<&Sweep> = volume
        .sweeps
        .iter()
        .filter(|s| s.radials.iter().any(|r| r.moments.contains_key(&moment)))
        .collect();
    let line = distance_bearing(start, end);

    let mut out = vec![CrossSectionCell::NoData; cols * rows];
    for col in 0..cols {
        let frac = if cols > 1 {
            col as f64 / (cols - 1) as f64
        } else {
            0.0
        };
        let point = radar_geo::destination_point(start, line.bearing_deg, line.distance_km * frac);
        let from_site = distance_bearing(site, point);

        // Moment on this column's radial, beam center height, half-width (km),
        // and slant range per sweep -- resolved once per column, not per cell.
        let beams: Vec<(&Moment, f64, f64, f64)> = sweeps
            .iter()
            .filter_map(|&sweep| {
                let elev = f64::from(sweep.elevation_angle_deg);
                let slant = slant_range_from_ground_range_km(from_site.distance_km, elev)?;
                let radial_index = find_radial_index_for_sweep(sweep, from_site.bearing_deg)?;
                let m = sweep.radials[radial_index].moments.get(&moment)?;
                let center = beam_height_km(slant, elev, site_height_km);
                let half = slant * (BEAMWIDTH_DEG / 2.0).to_radians().tan();
                Some((m, center, half, slant))
            })
            .collect();

        for row in 0..rows {
            let height = top_km * (1.0 - (row as f64 + 0.5) / rows as f64);
            let nearest = beams
                .iter()
                .filter(|(_, center, half, _)| (height - center).abs() <= *half)
                .min_by(|a, b| (height - a.1).abs().total_cmp(&(height - b.1).abs()));
            let Some(&(m, _, _, slant)) = nearest else {
                continue;
            };
            let cell = find_gate_index(m, slant).and_then(|g| m.gates.get(g));
            out[row * cols + col] = match cell {
                Some(GateValue::Value(v)) => CrossSectionCell::Value(*v),
                Some(GateValue::RangeFolded) => CrossSectionCell::RangeFolded,
                _ => CrossSectionCell::NoData,
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{
        AzimuthResolution, Moment, Radial, RadialStatus, RadialStatusKind, Site, Timestamp,
    };
    use std::collections::BTreeMap;

    /// One full 360-radial sweep at `elev` where every REF gate reads `value`.
    fn sweep(elev: f32, value: f32) -> Sweep {
        let radials = (0..360)
            .map(|az| {
                let mut moments = BTreeMap::new();
                moments.insert(
                    MomentKind::Reflectivity,
                    Moment {
                        first_gate_range_km: 2.125,
                        gate_spacing_km: 0.25,
                        scale: 2.0,
                        offset: 66.0,
                        gates: vec![GateValue::Value(value); 1800],
                    },
                );
                Radial {
                    azimuth_number: az as u16 + 1,
                    azimuth_angle_deg: az as f32,
                    azimuth_resolution: AzimuthResolution::One,
                    elevation_angle_deg: elev,
                    radial_status: RadialStatus {
                        kind: RadialStatusKind::Intermediate,
                        bad_data: false,
                    },
                    collection_time: Timestamp::from_epoch_millis(0),
                    moments,
                }
            })
            .collect();
        Sweep {
            elevation_number: 1,
            elevation_angle_deg: elev,
            radials,
        }
    }

    fn volume(sweeps: Vec<Sweep>) -> Volume {
        Volume {
            site: Site {
                icao: "KTST".into(),
                latitude_deg: 35.0,
                longitude_deg: -97.0,
                height_m: 0.0,
            },
            start_time: Timestamp::from_epoch_millis(0),
            volume_coverage_pattern: 212,
            sweeps,
        }
    }

    #[test]
    fn low_rows_take_the_low_tilt_high_rows_the_high_tilt_and_gaps_stay_empty() {
        let v = volume(vec![sweep(0.5, 10.0), sweep(10.0, 50.0)]);
        // A line due east of the site, 60-80 km out.
        let site = LatLon::new(35.0, -97.0);
        let a = radar_geo::destination_point(site, 90.0, 60.0);
        let b = radar_geo::destination_point(site, 90.0, 80.0);
        let rows = 100;
        let cells = cross_section(&v, MomentKind::Reflectivity, a, b, 2, rows, 20.0);
        let at = |km: f64| cells[(((20.0 - km) / 20.0) * rows as f64) as usize * 2];
        // 0.5 deg beam centered ~0.7-0.9 km up at 60 km.
        assert_eq!(at(0.8), CrossSectionCell::Value(10.0));
        // 10 deg beam centered ~10.8 km up at 60 km.
        assert_eq!(at(10.8), CrossSectionCell::Value(50.0));
        // Between the two tilts: no beam, no fabricated value.
        assert_eq!(at(5.0), CrossSectionCell::NoData);
        // Above everything.
        assert_eq!(at(19.5), CrossSectionCell::NoData);
    }

    #[test]
    fn sweeps_without_the_moment_are_ignored() {
        let v = volume(vec![sweep(0.5, 10.0)]);
        let site = LatLon::new(35.0, -97.0);
        let a = radar_geo::destination_point(site, 0.0, 50.0);
        let b = radar_geo::destination_point(site, 0.0, 60.0);
        let cells = cross_section(&v, MomentKind::Velocity, a, b, 4, 10, 10.0);
        assert!(cells.iter().all(|c| *c == CrossSectionCell::NoData));
    }
}
