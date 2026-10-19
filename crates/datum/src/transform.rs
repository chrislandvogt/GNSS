//! The transformation graph: published transformations between frames, and
//! explicit paths through them.
//!
//! Nodes are frames and edges are transformations, each with its method,
//! parameters, source document, validity area and uncertainty
//! (ARCHITECTURE.md, section 11.1). [`TransformationGraph::transform`] finds
//! the shortest path between two frames and epochs, applies it, and returns
//! every step so the path can be shown and recorded in provenance.
//!
//! Coordinates in a dynamic frame can only change epoch through a static
//! frame, using a plate motion model: for example ITRF2014 at 2018.0, to
//! GDA2020, to ITRF2014 at 2025.0. Outside the area a plate motion model
//! covers there's no way to change epoch, and the graph says so.
//!
//! Grid transformations, such as ICSM's GDA94 to GDA2020 conformal and
//! distortion grid, need the grid file, which the host supplies. Build a graph
//! that prefers them with [`TransformationGraph::with_preferred`].

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, OnceLock};

use nalgebra::{Matrix3, Vector3};

use crate::epoch::CoordinateEpoch;
use crate::error::DatumError;
use crate::frame::FrameId;
use crate::geodetic::{Geodetic, enu_rotation};
use crate::gridshift::HorizontalShiftGrid;
use crate::helmert::{Helmert7, Helmert14, HelmertParameters, RotationConvention};
use crate::position::{Position, describe, resolve_epoch};

/// A geographic bounding box where a transformation is valid, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    /// The area's name.
    pub name: &'static str,
    /// Southern limit of latitude.
    pub south_deg: f64,
    /// Northern limit of latitude.
    pub north_deg: f64,
    /// Western limit of longitude.
    pub west_deg: f64,
    /// Eastern limit of longitude.
    pub east_deg: f64,
}

impl Area {
    fn contains(&self, latitude_deg: f64, longitude_deg: f64) -> bool {
        (self.south_deg..=self.north_deg).contains(&latitude_deg)
            && (self.west_deg..=self.east_deg).contains(&longitude_deg)
    }
}

/// Australia and its offshore territories, as EPSG area 4177 bounds it.
pub const AUSTRALIA: Area = Area {
    name: "Australia including its offshore territories (EPSG area 4177)",
    south_deg: -60.55,
    north_deg: -8.47,
    west_deg: 93.41,
    east_deg: 173.34,
};

/// How a transformation changes coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    /// A 7-parameter similarity transformation.
    Helmert7(Helmert7),
    /// A time-dependent 14-parameter transformation, evaluated at the coordinate epoch.
    /// Between a dynamic and a static frame this is a plate motion model.
    Helmert14(Helmert14),
    /// The frames are aligned, so coordinates don't change, to within a tolerance.
    Identity {
        /// One-sigma difference between the frames, per axis, in metres.
        sigma_m: f64,
    },
    /// Latitude and longitude shifts from a grid, between two frames on the
    /// same ellipsoid.
    Grid(GridMethod),
}

/// A grid transformation: latitude and longitude from a shift grid, and
/// ellipsoidal heights from a similarity transformation, since the grid only
/// shifts horizontally.
#[derive(Debug, Clone)]
pub struct GridMethod {
    /// The shift grid.
    pub grid: Arc<HorizontalShiftGrid>,
    /// The similarity transformation that gives ellipsoidal heights, or `None`
    /// to leave heights unchanged. Its parameter uncertainty is also the
    /// uncertainty of the shift wherever the grid gives none.
    pub heights: Option<Helmert7>,
}

impl PartialEq for GridMethod {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.grid, &other.grid) && self.heights == other.heights
    }
}

/// One published transformation between two frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Transformation {
    /// The frame it transforms from, as published.
    pub from: FrameId,
    /// The frame it transforms to, as published.
    pub to: FrameId,
    /// The method and its parameters.
    pub method: Method,
    /// The transformation's name, for example "GDA94 to GDA2020 (1)".
    pub name: &'static str,
    /// Where the parameters come from.
    pub source: &'static str,
    /// Where it's valid, or `None` for everywhere.
    pub area: Option<Area>,
}

/// What one step of a path did to the coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    /// Nothing: the frames are aligned.
    Identity,
    /// This 7-parameter transformation. A 14-parameter one appears here
    /// evaluated at the coordinate epoch, and a reverse one with its
    /// parameters negated.
    Helmert(Helmert7),
    /// A shift from a grid.
    GridShift {
        /// The grid, as named when it was opened.
        grid: String,
        /// The subgrid used.
        subgrid: String,
        /// The change in latitude, in radians.
        latitude_change_rad: f64,
        /// The change in longitude, in radians.
        longitude_change_rad: f64,
        /// The change in ellipsoidal height, in metres.
        height_change_m: f64,
    },
}

/// One step of a transformation path, as applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The frame transformed from.
    pub from: FrameId,
    /// The coordinate epoch transformed from.
    pub from_epoch: CoordinateEpoch,
    /// The frame transformed to.
    pub to: FrameId,
    /// The coordinate epoch transformed to.
    pub to_epoch: CoordinateEpoch,
    /// The published transformation used.
    pub transformation: &'static str,
    /// Whether it was used in reverse.
    pub reversed: bool,
    /// Where its parameters come from.
    pub source: &'static str,
    /// What it did.
    pub applied: Applied,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} -> {}: {}{} ({})",
            describe(self.from, self.from_epoch),
            describe(self.to, self.to_epoch),
            if self.reversed { "inverse of " } else { "" },
            self.transformation,
            self.source
        )
    }
}

/// A transformed position, with the path that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Transformed {
    /// The position in the target frame and epoch. Its covariance, if it has
    /// one, is the source covariance propagated through each step.
    pub position: Position,
    /// The steps, in order. Empty if the source was already in the target frame and epoch.
    pub steps: Vec<Step>,
    /// The covariance (ECEF, m²) that uncertainty in the transformations
    /// themselves adds to the position. It's common to every position
    /// transformed along the same path, so it belongs with absolute
    /// uncertainty, not with vectors between positions. It's kept apart from
    /// [`Position::covariance`] so the caller decides.
    pub transformation_covariance: Matrix3<f64>,
}

/// Where a path is in its journey between epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Phase {
    /// In a dynamic frame, at the source epoch.
    SourceEpoch,
    /// In a static frame.
    Fixed,
    /// In a dynamic frame, at the target epoch.
    TargetEpoch,
}

/// A transformation used forwards or in reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Use {
    index: usize,
    reversed: bool,
}

/// A node of the path search: a frame, and where the path is between epochs.
type State = (FrameId, Phase);

/// A state the search has reached, and the state and transformation it was reached from.
struct Visit {
    state: State,
    from: Option<(State, Use)>,
}

/// A set of transformations between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformationGraph {
    transformations: Vec<Transformation>,
}

impl TransformationGraph {
    /// A graph of the given transformations. Paths prefer fewer steps, then earlier transformations.
    pub fn new(transformations: Vec<Transformation>) -> Self {
        Self { transformations }
    }

    /// The transformations this crate ships with: the ITRF, ATRF, GDA and WGS 84
    /// relationships needed in Australia. See [`builtin_transformations`].
    pub fn builtin() -> &'static TransformationGraph {
        static GRAPH: OnceLock<TransformationGraph> = OnceLock::new();
        GRAPH.get_or_init(|| TransformationGraph::new(builtin_transformations()))
    }

    /// The transformations in the graph.
    pub fn transformations(&self) -> &[Transformation] {
        &self.transformations
    }

    /// This graph with more transformations, which win over the ones already
    /// in it wherever both give a path of the same length: for example a grid
    /// preferred over a similarity transformation between the same frames.
    /// A preferred transformation is only used where it's valid, so outside a
    /// grid the others still apply.
    pub fn with_preferred(&self, preferred: Vec<Transformation>) -> TransformationGraph {
        let mut transformations = preferred;
        transformations.extend(self.transformations.iter().cloned());
        TransformationGraph { transformations }
    }

    /// Transforms a position to a frame and, for a dynamic frame, a coordinate epoch.
    pub fn transform(
        &self,
        position: &Position,
        to: FrameId,
        epoch: Option<CoordinateEpoch>,
    ) -> Result<Transformed, DatumError> {
        let target_epoch = resolve_epoch(to, epoch)?;
        // Only transformations valid where the position is can be on the path.
        let here = position.geodetic();
        let usable: Vec<bool> = self
            .transformations
            .iter()
            .map(|t| {
                let in_area = t
                    .area
                    .is_none_or(|a| a.contains(here.latitude_deg(), here.longitude_deg()));
                let covered = match &t.method {
                    Method::Grid(g) => g.grid.covers(here.latitude_rad(), here.longitude_rad()),
                    _ => true,
                };
                in_area && covered
            })
            .collect();
        let path = self.find_path(
            position.frame(),
            position.epoch(),
            to,
            target_epoch,
            &usable,
        )?;

        let mut frame = position.frame();
        let mut current_epoch = position.epoch();
        let mut xyz = *position.xyz();
        let mut covariance = position.covariance().copied();
        let mut transformation_covariance = Matrix3::zeros();
        let mut steps = Vec::with_capacity(path.len());

        for u in path {
            let t = &self.transformations[u.index];
            let next = if u.reversed { t.from } else { t.to };
            let next_epoch = match next.reference_epoch() {
                Some(reference) => reference,
                // Leaving a static frame for a dynamic one lands on the target epoch.
                None if frame.is_static() => target_epoch,
                None => current_epoch,
            };
            if let Some(area) = t.area {
                let g = frame.ellipsoid().ecef_to_geodetic(&xyz)?;
                if !area.contains(g.latitude_deg(), g.longitude_deg()) {
                    return Err(DatumError::OutsideArea {
                        transformation: t.name,
                        area: area.name,
                        latitude_deg: g.latitude_deg(),
                        longitude_deg: g.longitude_deg(),
                    });
                }
            }
            let helmert = match &t.method {
                Method::Helmert7(h) => Some(if u.reversed { h.inverse() } else { *h }),
                Method::Helmert14(h) => {
                    // Evaluated at the epoch of whichever side is dynamic.
                    let at = if frame.is_static() {
                        next_epoch
                    } else {
                        current_epoch
                    };
                    Some(if u.reversed { h.inverse() } else { *h }.at(at))
                }
                Method::Identity { .. } | Method::Grid(_) => None,
            };
            let applied = match (&t.method, helmert) {
                (_, Some(h)) => {
                    let jacobian = h.jacobian();
                    transformation_covariance =
                        jacobian * transformation_covariance * jacobian.transpose()
                            + h.parameter_covariance(&xyz);
                    covariance = covariance.map(|c| c.propagate(&jacobian));
                    xyz = h.apply(&xyz);
                    Applied::Helmert(h)
                }
                (Method::Identity { sigma_m }, None) => {
                    transformation_covariance += Matrix3::from_diagonal_element(sigma_m * sigma_m);
                    Applied::Identity
                }
                (Method::Grid(g), None) => {
                    let shifted = apply_grid(g, frame, &xyz, u.reversed)?;
                    let jacobian = shifted.jacobian;
                    transformation_covariance =
                        jacobian * transformation_covariance * jacobian.transpose()
                            + shifted.added_covariance;
                    covariance = covariance.map(|c| c.propagate(&jacobian));
                    xyz = shifted.xyz;
                    shifted.applied
                }
                (_, None) => Applied::Identity,
            };
            steps.push(Step {
                from: frame,
                from_epoch: current_epoch,
                to: next,
                to_epoch: next_epoch,
                transformation: t.name,
                reversed: u.reversed,
                source: t.source,
                applied,
            });
            frame = next;
            current_epoch = next_epoch;
        }

        Ok(Transformed {
            position: Position::new(frame, Some(current_epoch), xyz, covariance)?,
            steps,
            transformation_covariance,
        })
    }

    /// The shortest path between two frames and epochs, by breadth-first search.
    fn find_path(
        &self,
        from: FrameId,
        from_epoch: CoordinateEpoch,
        to: FrameId,
        to_epoch: CoordinateEpoch,
        usable: &[bool],
    ) -> Result<Vec<Use>, DatumError> {
        let start = (
            from,
            if from.is_static() {
                Phase::Fixed
            } else {
                Phase::SourceEpoch
            },
        );
        let same_epoch = from_epoch == to_epoch;
        let is_goal = |(frame, phase): State| {
            frame == to
                && match phase {
                    Phase::Fixed => to.is_static(),
                    Phase::TargetEpoch => true,
                    Phase::SourceEpoch => same_epoch,
                }
        };

        let mut visited = vec![Visit {
            state: start,
            from: None,
        }];
        let mut queue = VecDeque::from([start]);
        while let Some(state) = queue.pop_front() {
            if is_goal(state) {
                return Ok(unwind(&visited, state));
            }
            for (index, t) in self.transformations.iter().enumerate() {
                if !usable[index] {
                    continue;
                }
                for reversed in [false, true] {
                    let (a, b) = if reversed {
                        (t.to, t.from)
                    } else {
                        (t.from, t.to)
                    };
                    if a != state.0 {
                        continue;
                    }
                    let Some(phase) = next_phase(state.1, b, to.is_static()) else {
                        continue;
                    };
                    let next = (b, phase);
                    if visited.iter().any(|v| v.state == next) {
                        continue;
                    }
                    visited.push(Visit {
                        state: next,
                        from: Some((state, Use { index, reversed })),
                    });
                    queue.push_back(next);
                }
            }
        }

        let hint = if !from.is_static() && !to.is_static() && !same_epoch {
            ". Changing the epoch of a dynamic frame needs a plate motion model, \
             and none covers this position"
        } else {
            ". Some transformations are only defined in part of the world; \
             check the position is where you expect"
        };
        Err(DatumError::NoPath {
            from: describe(from, from_epoch),
            to: describe(to, to_epoch),
            hint: hint.to_owned(),
        })
    }
}

/// The phase after moving into `next`, or `None` if the move isn't allowed.
fn next_phase(phase: Phase, next: FrameId, target_is_static: bool) -> Option<Phase> {
    match (phase, next.is_static()) {
        (Phase::SourceEpoch, false) => Some(Phase::SourceEpoch),
        (Phase::SourceEpoch | Phase::Fixed, true) => Some(Phase::Fixed),
        // Leaving a static frame needs a target epoch.
        (Phase::Fixed, false) if !target_is_static => Some(Phase::TargetEpoch),
        (Phase::Fixed, false) => None,
        (Phase::TargetEpoch, false) => Some(Phase::TargetEpoch),
        // Back into a static frame would undo the epoch change.
        (Phase::TargetEpoch, true) => None,
    }
}

/// The path to `end`, from the breadth-first search's record of where each state came from.
fn unwind(visited: &[Visit], end: State) -> Vec<Use> {
    let mut path = Vec::new();
    let mut state = end;
    while let Some((previous, used)) = visited
        .iter()
        .find(|v| v.state == state)
        .and_then(|v| v.from)
    {
        path.push(used);
        state = previous;
    }
    path.reverse();
    path
}

/// A grid step's result.
struct GridStep {
    xyz: Vector3<f64>,
    jacobian: Matrix3<f64>,
    added_covariance: Matrix3<f64>,
    applied: Applied,
}

/// Applies a grid transformation to ECEF coordinates in `frame`: latitude and
/// longitude from the grid (in reverse, by iteration), and ellipsoidal height
/// from the similarity transformation, if there is one.
fn apply_grid(
    g: &GridMethod,
    frame: FrameId,
    xyz: &Vector3<f64>,
    reversed: bool,
) -> Result<GridStep, DatumError> {
    let ellipsoid = frame.ellipsoid();
    let here = ellipsoid.ecef_to_geodetic(xyz)?;
    let (lat, lon, shift) = if reversed {
        g.grid.inverse(here.latitude_rad(), here.longitude_rad())?
    } else {
        g.grid.forward(here.latitude_rad(), here.longitude_rad())?
    };
    let helmert = g.heights.map(|h| if reversed { h.inverse() } else { h });
    let height = match helmert {
        Some(h) => ellipsoid
            .ecef_to_geodetic(&h.apply(xyz))?
            .ellipsoidal_height_m(),
        None => here.ellipsoidal_height_m(),
    };
    let there = Geodetic::new(lat, lon, height)?;

    // The shift changes by well under a millionth of a metre per metre, so the
    // step's Jacobian is the similarity transformation's, or the identity.
    let jacobian = helmert.map_or_else(Matrix3::identity, |h| h.jacobian());
    // The conformal part's uncertainty applies everywhere. Where the grid
    // gives an accuracy, the distortion model's uncertainty adds to it.
    let mut added = helmert.map_or_else(Matrix3::zeros, |h| h.parameter_covariance(xyz));
    if let (Some(s_lat), Some(s_lon)) = (shift.latitude_sigma_rad, shift.longitude_sigma_rad) {
        let north_m = s_lat * (ellipsoid.meridian_radius_m(lat) + height);
        let east_m = s_lon * (ellipsoid.prime_vertical_radius_m(lat) + height) * libm::cos(lat);
        let r = enu_rotation(lat, lon);
        let enu = Matrix3::from_diagonal(&Vector3::new(east_m * east_m, north_m * north_m, 0.0));
        added += r.transpose() * enu * r;
    }
    let subgrid = g
        .grid
        .grid()
        .subgrids()
        .get(shift.subgrid)
        .map_or_else(String::new, |s| s.name.clone());
    Ok(GridStep {
        xyz: ellipsoid.geodetic_to_ecef(&there),
        jacobian,
        added_covariance: added,
        applied: Applied::GridShift {
            grid: g.grid.grid().name().to_owned(),
            subgrid,
            latitude_change_rad: lat - here.latitude_rad(),
            longitude_change_rad: lon - here.longitude_rad(),
            height_change_m: height - here.ellipsoidal_height_m(),
        },
    })
}

/// GDA94 to GDA2020 (1), EPSG:8048: the conformal 7-parameter transformation
/// of the GDA2020 Technical Manual v1.2, section 3.1.
fn gda94_to_gda2020_conformal() -> Helmert7 {
    Helmert7 {
        parameters: HelmertParameters::from_mm_mas_ppb(
            [61.55, -10.87, -40.19],
            [-39.4924, -32.7221, -32.8979],
            -9.994,
        ),
        sigmas: Some(HelmertParameters::from_mm_mas_ppb(
            [0.7, 0.6, 0.7],
            [0.011, 0.010, 0.011],
            0.10,
        )),
        convention: RotationConvention::CoordinateFrame,
    }
}

/// GDA94 to GDA2020 (2), EPSG:8447: ICSM's conformal and distortion grid.
/// It's for GDA94 coordinates that carry the distortion of the state and
/// territory survey networks; see the GDA2020 Technical Manual and ICSM's fact
/// sheet T1 for when to use it rather than the conformal transformation.
///
/// `grid` is ICSM's `GDA94_GDA2020_conformal_and_distortion.gsb` or PROJ's
/// `au_icsm_GDA94_GDA2020_conformal_and_distortion.tif`. It covers mainland
/// Australia and Tasmania. The grid is 2D, so ellipsoidal heights come from
/// the conformal transformation (EPSG:8048).
pub fn gda94_to_gda2020_conformal_and_distortion(
    grid: Arc<HorizontalShiftGrid>,
) -> Result<Transformation, DatumError> {
    let g = grid.grid();
    let meta = g.metadata();
    // Lenient about spelling (GDA94, GDA_94, GDA 1994), but enough to catch
    // the wrong grid, such as a geoid model.
    let is_gda94_to_gda2020 = match g.format() {
        crate::grid::GridFormat::Ntv2 => {
            let from = meta.get("SYSTEM_F").unwrap_or("");
            let to = meta.get("SYSTEM_T").unwrap_or("");
            from.contains("94") && !from.contains("2020") && to.contains("2020")
        }
        crate::grid::GridFormat::GeoTiff => meta.get("target_crs_epsg_code") == Some("7844"),
    };
    if !is_gda94_to_gda2020 {
        return Err(crate::grid::invalid(
            g.name(),
            "isn't a GDA94 to GDA2020 grid: its header doesn't relate GDA94 to GDA2020 \
             (NTv2 SYSTEM_F and SYSTEM_T) or target EPSG:7844 (GeoTIFF)",
        ));
    }
    Ok(Transformation {
        from: FrameId::Gda94,
        to: FrameId::Gda2020,
        method: Method::Grid(GridMethod {
            grid,
            heights: Some(gda94_to_gda2020_conformal()),
        }),
        name: "GDA94 to GDA2020 (2)",
        source: "EPSG:8447, ICSM conformal and distortion grid; ellipsoidal heights by EPSG:8048",
        area: Some(AUSTRALIA),
    })
}

/// The transformations this crate ships with.
///
/// Parameters are as EPSG publishes them (database v12.029, read through
/// PROJ 9.8.1), checked against GeodePy's constants. Uncertainties are the
/// one-sigma values from the GDA2020 Technical Manual and the IERS (via
/// GeodePy); for identities, EPSG's accuracy is used as a one-sigma value
/// per axis, which is conservative.
pub fn builtin_transformations() -> Vec<Transformation> {
    use crate::helmert::MAS;
    let mas = |rx: f64, ry: f64, rz: f64| nalgebra::Vector3::new(rx, ry, rz) * MAS;

    let plate_motion = Helmert14 {
        parameters: HelmertParameters::zero(),
        rates: HelmertParameters::from_mm_mas_ppb([0.0; 3], [1.50379, 1.18346, 1.20716], 0.0),
        reference_epoch: CoordinateEpoch::from_const(2020.0),
        sigmas: None,
        rate_sigmas: Some(HelmertParameters {
            translation_m: nalgebra::Vector3::zeros(),
            rotation_rad: mas(0.00417, 0.00401, 0.00370),
            scale: 0.0,
        }),
        convention: RotationConvention::CoordinateFrame,
    };

    vec![
        Transformation {
            from: FrameId::Itrf2014,
            to: FrameId::Itrf2020,
            method: Method::Helmert14(Helmert14 {
                parameters: HelmertParameters::from_mm_mas_ppb([1.4, 0.9, -1.4], [0.0; 3], 0.42),
                rates: HelmertParameters::from_mm_mas_ppb([0.0, 0.1, -0.2], [0.0; 3], 0.0),
                reference_epoch: CoordinateEpoch::from_const(2015.0),
                sigmas: Some(HelmertParameters::from_mm_mas_ppb(
                    [0.2, 0.2, 0.2],
                    [0.007, 0.006, 0.007],
                    0.03,
                )),
                rate_sigmas: Some(HelmertParameters::from_mm_mas_ppb(
                    [0.2, 0.2, 0.2],
                    [0.007, 0.006, 0.007],
                    0.03,
                )),
                convention: RotationConvention::PositionVector,
            }),
            name: "ITRF2014 to ITRF2020 (1)",
            source: "EPSG:9991, IERS ITRF2020 parameters",
            area: None,
        },
        Transformation {
            from: FrameId::Igs20,
            to: FrameId::Itrf2020,
            method: Method::Identity { sigma_m: 0.0 },
            name: "IGS20 = ITRF2020",
            source: "IGS20 is the IGS realisation of ITRF2020; EPSG publishes no transformation",
            area: None,
        },
        Transformation {
            from: FrameId::Itrf2014,
            to: FrameId::Atrf2014,
            method: Method::Identity { sigma_m: 0.01 },
            name: "ITRF2014 to ATRF2014 (1)",
            source: "EPSG:9460",
            area: Some(AUSTRALIA),
        },
        Transformation {
            from: FrameId::Atrf2014,
            to: FrameId::Gda2020,
            method: Method::Helmert14(plate_motion),
            name: "ATRF2014 to GDA2020 (1)",
            source: "EPSG:9459, Australian Plate Motion Model (GDA2020 Technical Manual v1.2)",
            area: Some(AUSTRALIA),
        },
        Transformation {
            from: FrameId::Itrf2014,
            to: FrameId::Gda2020,
            method: Method::Helmert14(plate_motion),
            name: "ITRF2014 to GDA2020 (1)",
            source: "EPSG:8049, Australian Plate Motion Model (GDA2020 Technical Manual v1.2)",
            area: Some(AUSTRALIA),
        },
        Transformation {
            from: FrameId::Gda94,
            to: FrameId::Gda2020,
            method: Method::Helmert7(gda94_to_gda2020_conformal()),
            name: "GDA94 to GDA2020 (1)",
            source: "EPSG:8048, GDA2020 Technical Manual v1.2 section 3.1 (conformal)",
            area: Some(AUSTRALIA),
        },
        Transformation {
            from: FrameId::Wgs84G2296,
            to: FrameId::Itrf2020,
            method: Method::Identity { sigma_m: 0.01 },
            name: "WGS 84 (G2296) to ITRF2020 (1)",
            source: "EPSG:10608",
            area: None,
        },
        Transformation {
            from: FrameId::Wgs84G2139,
            to: FrameId::Itrf2014,
            method: Method::Identity { sigma_m: 0.01 },
            name: "WGS 84 (G2139) to ITRF2014 (1)",
            source: "EPSG:9757",
            area: None,
        },
        Transformation {
            from: FrameId::Wgs84,
            to: FrameId::Wgs84G2139,
            method: Method::Identity { sigma_m: 2.0 },
            name: "WGS 84 ensemble to WGS 84 (G2139)",
            source: "EPSG datum ensemble 6326, accuracy 2 m",
            area: None,
        },
    ]
}

/// Convenience for [`TransformationGraph::transform`] on the built-in graph.
pub fn transform(
    position: &Position,
    to: FrameId,
    epoch: Option<CoordinateEpoch>,
) -> Result<Transformed, DatumError> {
    TransformationGraph::builtin().transform(position, to, epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helmert::MM;
    use nalgebra::Vector3;

    /// Published coordinates are rounded to a tenth of a millimetre.
    const TENTH_MM: f64 = 0.1 * MM;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn epoch(y: f64) -> CoordinateEpoch {
        CoordinateEpoch::from_decimal_year(y).unwrap()
    }

    fn pos(frame: FrameId, e: Option<f64>, xyz: [f64; 3]) -> Position {
        Position::new(frame, e.map(epoch), Vector3::from(xyz), None).unwrap()
    }

    // GDA2020 Technical Manual v1.2, worked examples for ALIC (Alice Springs),
    // as replicated in GeodePy's tests. Published to 0.1 mm.
    const ALIC_GDA94: [f64; 3] = [-4052051.7643, 4212836.2017, -2545106.0245];
    const ALIC_GDA2020_FROM_GDA94: [f64; 3] = [-4052052.7379, 4212835.9897, -2545104.5898];
    const ALIC_GDA2020: [f64; 3] = [-4052052.7373, 4212835.9835, -2545104.5867];
    const ALIC_ITRF2014_2018: [f64; 3] = [-4052052.6588, 4212835.9938, -2545104.6946];

    fn assert_close(actual: &Vector3<f64>, expected: [f64; 3], tolerance_m: f64) {
        let d = actual - Vector3::from(expected);
        assert!(d.abs().max() <= tolerance_m, "differs by {d:?} m");
    }

    #[test]
    fn manual_example_gda94_to_gda2020() {
        let t = transform(
            &pos(FrameId::Gda94, None, ALIC_GDA94),
            FrameId::Gda2020,
            None,
        )
        .unwrap();
        // Half the published rounding, plus a hair for our own rounding.
        assert_close(
            t.position.xyz(),
            ALIC_GDA2020_FROM_GDA94,
            0.5 * TENTH_MM + 1e-9,
        );
        assert_eq!(t.steps.len(), 1);
        let back = transform(&t.position, FrameId::Gda94, None).unwrap();
        assert_close(back.position.xyz(), ALIC_GDA94, 1e-6);
    }

    #[test]
    fn manual_example_itrf2014_2018_to_gda2020() {
        let itrf = pos(FrameId::Itrf2014, Some(2018.0), ALIC_ITRF2014_2018);
        let t = transform(&itrf, FrameId::Gda2020, None).unwrap();
        assert_close(t.position.xyz(), ALIC_GDA2020, 0.5 * TENTH_MM);
        let back = transform(&t.position, FrameId::Itrf2014, Some(epoch(2018.0))).unwrap();
        assert_close(back.position.xyz(), ALIC_ITRF2014_2018, 0.5 * TENTH_MM);
        assert_eq!(back.position.frame_and_epoch(), "ITRF2014 at 2018.0");
    }

    #[test]
    fn epoch_changes_go_through_a_static_frame() {
        let p = pos(FrameId::Atrf2014, Some(2018.0), ALIC_ITRF2014_2018);
        let t = transform(&p, FrameId::Atrf2014, Some(epoch(2025.0))).unwrap();
        let path: Vec<_> = t.steps.iter().map(|s| describe(s.to, s.to_epoch)).collect();
        assert_eq!(path, ["GDA2020", "ATRF2014 at 2025.0"]);
        // About 67 mm/yr north-north-east over seven years.
        let moved = (t.position.xyz() - p.xyz()).norm();
        assert!((moved - 0.47).abs() < 0.01, "{moved}");
    }

    #[test]
    fn itrf2020_to_gda2020_goes_through_itrf2014() {
        let p = pos(FrameId::Itrf2020, Some(2023.5), ALIC_ITRF2014_2018);
        let t = transform(&p, FrameId::Gda2020, None).unwrap();
        let names: Vec<_> = t.steps.iter().map(|s| s.transformation).collect();
        assert_eq!(
            names,
            ["ITRF2014 to ITRF2020 (1)", "ITRF2014 to GDA2020 (1)"]
        );
        assert!(t.steps[0].reversed);
    }

    #[test]
    fn same_frame_and_epoch_is_a_no_op() {
        let p = pos(FrameId::Itrf2014, Some(2018.0), ALIC_ITRF2014_2018);
        let t = transform(&p, FrameId::Itrf2014, Some(epoch(2018.0))).unwrap();
        assert!(t.steps.is_empty());
        assert_eq!(t.position, p);
    }

    #[test]
    fn epoch_changes_outside_australia_have_no_path() {
        // Kiruna, Sweden: no plate motion model here.
        let kiruna = [2_251_420.0, 862_817.0, 5_885_476.0];
        let p = pos(FrameId::Itrf2020, Some(2018.0), kiruna);
        let err = transform(&p, FrameId::Itrf2020, Some(epoch(2025.0))).unwrap_err();
        assert!(matches!(err, DatumError::NoPath { .. }), "{err}");
        assert!(err.to_string().contains("plate motion model"), "{err}");
        // Same epoch is fine.
        assert!(transform(&p, FrameId::Itrf2014, Some(epoch(2018.0))).is_ok());
    }

    #[test]
    fn wgs84_ensemble_carries_two_metres() {
        let p = pos(FrameId::Wgs84, Some(2024.0), ALIC_GDA2020);
        let t = transform(&p, FrameId::Itrf2014, Some(epoch(2024.0))).unwrap();
        assert_eq!(t.steps.len(), 2);
        let sigma = libm::sqrt(t.transformation_covariance[(0, 0)]);
        assert!((sigma - libm::sqrt(4.0 + 1e-4)).abs() < 1e-12, "{sigma}");
    }

    #[test]
    fn static_target_with_wrong_epoch_is_refused() {
        let p = pos(FrameId::Itrf2014, Some(2018.0), ALIC_ITRF2014_2018);
        assert!(matches!(
            transform(&p, FrameId::Gda2020, Some(epoch(2018.0))),
            Err(DatumError::EpochOnStaticFrame { .. })
        ));
    }

    /// A GDA94 to GDA2020 grid around Alice Springs whose shifts are the
    /// conformal transformation's own, as in ICSM's conformal-only grid
    /// (EPSG:8446), so the grid path must reproduce the conformal one.
    fn conformal_grid() -> Arc<HorizontalShiftGrid> {
        use crate::ellipsoid::GRS80;
        use crate::grid::Grid;
        use crate::grid::tests::{TestSubgrid, ntv2_bytes};
        let conformal = gda94_to_gda2020_conformal();
        let values = move |lat: f64, lon: f64| {
            let g = Geodetic::from_degrees(lat, lon, 0.0).unwrap();
            let moved = GRS80
                .ecef_to_geodetic(&conformal.apply(&GRS80.geodetic_to_ecef(&g)))
                .unwrap();
            let arc_seconds = |rad: f64| rad.to_degrees() * 3600.0;
            [
                arc_seconds(moved.latitude_rad() - g.latitude_rad()) as f32,
                // NTv2 longitude shifts are positive west.
                -arc_seconds(moved.longitude_rad() - g.longitude_rad()) as f32,
                0.0001,
                0.0002,
            ]
        };
        let bytes = ntv2_bytes(
            &[TestSubgrid {
                name: "ALIC",
                parent: "NONE",
                s_lat: -24.5 * 3600.0,
                n_lat: -23.0 * 3600.0,
                e_long: -134.5 * 3600.0,
                w_long: -133.0 * 3600.0,
                lat_inc: 180.0,
                long_inc: 180.0,
                values: &values,
            }],
            true,
        );
        let grid = Grid::open("conformal-test.gsb", Arc::new(bytes)).unwrap();
        Arc::new(HorizontalShiftGrid::new(grid).unwrap())
    }

    fn graph_with_grid() -> TransformationGraph {
        let t = gda94_to_gda2020_conformal_and_distortion(conformal_grid()).unwrap();
        TransformationGraph::builtin().with_preferred(vec![t])
    }

    #[test]
    fn a_grid_of_the_conformal_shifts_reproduces_them() {
        let graph = graph_with_grid();
        let p = pos(FrameId::Gda94, None, ALIC_GDA94);
        let by_grid = graph.transform(&p, FrameId::Gda2020, None).unwrap();
        assert_eq!(by_grid.steps[0].transformation, "GDA94 to GDA2020 (2)");
        let by_helmert = transform(&p, FrameId::Gda2020, None).unwrap();
        // Bilinear interpolation of a smooth field, from f32 nodes: micrometres.
        let d = (by_grid.position.xyz() - by_helmert.position.xyz()).norm();
        assert!(d < 1e-5, "{d}");
        assert_close(
            by_grid.position.xyz(),
            ALIC_GDA2020_FROM_GDA94,
            0.5 * TENTH_MM + 1e-5,
        );

        let back = graph
            .transform(&by_grid.position, FrameId::Gda94, None)
            .unwrap();
        assert_eq!(back.steps[0].transformation, "GDA94 to GDA2020 (2)");
        assert!(back.steps[0].reversed);
        // Latitude and longitude return exactly; heights go through the
        // conformal transformation and its EPSG reverse, which negates the
        // parameters and so misses an exact inverse by a fraction of a
        // micrometre, as in the Helmert round trip above.
        let d = (back.position.xyz() - p.xyz()).norm();
        assert!(d < 1e-6, "{d}");
    }

    #[test]
    fn grid_steps_record_the_shift_and_heights_follow_the_conformal_transformation() {
        let p = pos(FrameId::Gda94, None, ALIC_GDA94);
        let t = graph_with_grid()
            .transform(&p, FrameId::Gda2020, None)
            .unwrap();
        let Applied::GridShift {
            grid,
            subgrid,
            latitude_change_rad,
            height_change_m,
            ..
        } = &t.steps[0].applied
        else {
            panic!("expected a grid shift, got {:?}", t.steps[0].applied);
        };
        assert_eq!(
            (grid.as_str(), subgrid.as_str()),
            ("conformal-test.gsb", "ALIC")
        );
        // About 1.5 m north.
        let north_m = latitude_change_rad * 6.36e6;
        assert!((north_m - 1.5).abs() < 0.1, "{north_m}");
        let conformal = transform(&p, FrameId::Gda2020, None).unwrap();
        let expected = conformal.position.geodetic().ellipsoidal_height_m()
            - p.geodetic().ellipsoidal_height_m();
        assert!((height_change_m - expected).abs() < 1e-9);
    }

    #[test]
    fn the_grids_accuracy_adds_to_the_conformal_uncertainty() {
        let p = pos(FrameId::Gda94, None, ALIC_GDA94);
        let g = p.geodetic();
        let by_grid = graph_with_grid()
            .transform(&p, FrameId::Gda2020, None)
            .unwrap();
        let by_helmert = transform(&p, FrameId::Gda2020, None).unwrap();
        let r = g.enu_rotation();
        let extra = r
            * (by_grid.transformation_covariance - by_helmert.transformation_covariance)
            * r.transpose();
        // 0.0002" east and 0.0001" north, at 30.9 m per arc-second of latitude.
        let arc_second_m = 30.9;
        let east = 0.0002 * arc_second_m * libm::cos(g.latitude_rad());
        let north = 0.0001 * arc_second_m;
        assert!((libm::sqrt(extra[(0, 0)]) - east).abs() < 0.01 * east);
        assert!((libm::sqrt(extra[(1, 1)]) - north).abs() < 0.01 * north);
        assert!(extra[(2, 2)].abs() < 1e-12);
    }

    #[test]
    fn outside_the_grid_the_conformal_transformation_applies() {
        let perth = Position::from_geodetic(
            FrameId::Gda94,
            None,
            &Geodetic::from_degrees(-31.95, 115.86, 10.0).unwrap(),
            None,
        )
        .unwrap();
        let t = graph_with_grid()
            .transform(&perth, FrameId::Gda2020, None)
            .unwrap();
        assert_eq!(t.steps[0].transformation, "GDA94 to GDA2020 (1)");
        assert!(matches!(t.steps[0].applied, Applied::Helmert(_)));
    }

    #[test]
    fn only_a_gda94_to_gda2020_grid_is_accepted() {
        use crate::grid::Grid;
        use crate::grid::tests::{TestSubgrid, ntv2_bytes, plane};
        let mut bytes = ntv2_bytes(
            &[TestSubgrid {
                name: "X",
                parent: "NONE",
                s_lat: -36.0 * 3600.0,
                n_lat: -35.0 * 3600.0,
                e_long: -146.0 * 3600.0,
                w_long: -145.0 * 3600.0,
                lat_inc: 3600.0,
                long_inc: 3600.0,
                values: &plane,
            }],
            true,
        );
        // SYSTEM_T becomes AGD66.
        bytes[16 * 6 + 8..16 * 6 + 16].copy_from_slice(b"AGD66   ");
        let grid =
            HorizontalShiftGrid::new(Grid::open("agd.gsb", Arc::new(bytes)).unwrap()).unwrap();
        let err = gda94_to_gda2020_conformal_and_distortion(Arc::new(grid)).unwrap_err();
        assert!(
            err.to_string().contains("isn't a GDA94 to GDA2020 grid"),
            "{err}"
        );
    }

    #[test]
    fn steps_describe_themselves() {
        let p = pos(FrameId::Gda94, None, ALIC_GDA94);
        let t = transform(&p, FrameId::Atrf2014, Some(epoch(2018.0))).unwrap();
        let text: Vec<String> = t.steps.iter().map(ToString::to_string).collect();
        assert_eq!(
            text,
            [
                "GDA94 -> GDA2020: GDA94 to GDA2020 (1) (EPSG:8048, GDA2020 Technical Manual v1.2 section 3.1 (conformal))",
                "GDA2020 -> ATRF2014 at 2018.0: inverse of ATRF2014 to GDA2020 (1) (EPSG:9459, Australian Plate Motion Model (GDA2020 Technical Manual v1.2))",
            ]
        );
    }
}
