//! Errors from datum operations.

use crate::epoch::CoordinateEpoch;
use crate::frame::FrameId;

/// Everything that can go wrong in this crate. Each variant says what was
/// wrong and, where it helps, what to do instead.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DatumError {
    /// A number that has to be finite wasn't.
    #[error("{what} must be a finite number, got {value}")]
    NotFinite {
        /// What the number is, for example "X coordinate".
        what: &'static str,
        /// The value given.
        value: f64,
    },

    /// A decimal year outside the range this crate handles.
    #[error("coordinate epoch {value} is outside the supported range 1900.0 to 2200.0")]
    EpochOutOfRange {
        /// The decimal year given.
        value: f64,
    },

    /// A time that can't be converted to or from a calendar date.
    #[error("time can't be converted to a calendar date: {reason}")]
    InvalidTime {
        /// What went wrong.
        reason: String,
    },

    /// A dynamic frame was given without a coordinate epoch.
    #[error("{frame} is a dynamic frame, so a position in it needs a coordinate epoch")]
    MissingEpoch {
        /// The frame.
        frame: FrameId,
    },

    /// A static frame was given an epoch other than its reference epoch.
    #[error(
        "{frame} is a static frame: its coordinates are fixed at its reference epoch {reference}. \
         A coordinate at epoch {given} belongs in a dynamic frame such as ATRF2014 or ITRF2014; \
         transform it to {frame} instead"
    )]
    EpochOnStaticFrame {
        /// The static frame.
        frame: FrameId,
        /// The frame's reference epoch.
        reference: CoordinateEpoch,
        /// The epoch that was given.
        given: CoordinateEpoch,
    },

    /// Two positions were combined without transforming them to a common frame and epoch.
    #[error(
        "can't combine a position in {a} with one in {b}: transform one of them to the other's \
         frame and epoch first"
    )]
    FrameMismatch {
        /// The first position's frame and epoch.
        a: String,
        /// The second position's frame and epoch.
        b: String,
    },

    /// A name that isn't a known frame.
    #[error("unknown frame {name:?}. Known frames: {known}")]
    UnknownFrame {
        /// The name given.
        name: String,
        /// The names this crate knows.
        known: String,
    },

    /// No chain of transformations connects the frames and epochs asked for.
    #[error("no transformation path from {from} to {to}{hint}")]
    NoPath {
        /// The source frame and epoch.
        from: String,
        /// The target frame and epoch.
        to: String,
        /// Extra explanation, if there is one.
        hint: String,
    },

    /// A position is outside the area where a transformation is defined.
    #[error(
        "{transformation} is only defined in {area}, but the position is at latitude \
         {latitude_deg:.4}°, longitude {longitude_deg:.4}°"
    )]
    OutsideArea {
        /// The transformation.
        transformation: &'static str,
        /// The area where it's valid.
        area: &'static str,
        /// Latitude of the position, in degrees.
        latitude_deg: f64,
        /// Longitude of the position, in degrees.
        longitude_deg: f64,
    },

    /// An ECEF position that isn't near the Earth's surface.
    #[error(
        "ECEF position ({x_m:.3}, {y_m:.3}, {z_m:.3}) m is {radius_km:.1} km from the geocentre; \
         positions must be between 5,000 and 10,000 km from it (near the Earth's surface)"
    )]
    NotNearSurface {
        /// X, in metres.
        x_m: f64,
        /// Y, in metres.
        y_m: f64,
        /// Z, in metres.
        z_m: f64,
        /// Distance from the geocentre, in kilometres.
        radius_km: f64,
    },

    /// A latitude outside ±90°.
    #[error("latitude {latitude_deg}° is outside -90° to 90°")]
    InvalidLatitude {
        /// The latitude given, in degrees.
        latitude_deg: f64,
    },

    /// A covariance matrix that can't be one.
    #[error("invalid covariance matrix: {reason}")]
    InvalidCovariance {
        /// What's wrong with it.
        reason: String,
    },

    /// An MGA zone number out of range.
    #[error("MGA zone {zone} is outside 1 to 60")]
    InvalidZone {
        /// The zone given.
        zone: u8,
    },

    /// A point too far from a projection's central meridian for its formulas.
    #[error(
        "longitude is {offset_deg:.3}° from the central meridian; Transverse Mercator \
         coordinates are only computed within {limit_deg}° of it"
    )]
    TooFarFromCentralMeridian {
        /// The longitude difference, in degrees.
        offset_deg: f64,
        /// The limit, in degrees.
        limit_deg: f64,
    },

    /// A projection that doesn't belong to the frame of the position.
    #[error(
        "{projection} grid coordinates are defined on {expected}, but the position is in {frame}"
    )]
    WrongFrameForProjection {
        /// The projection, for example "MGA2020".
        projection: &'static str,
        /// The frame it's defined on.
        expected: FrameId,
        /// The position's frame.
        frame: FrameId,
    },

    /// A grid file that can't be read as the format it claims to be, or that
    /// doesn't hold what it's being used for.
    #[error("{grid}: {reason}")]
    InvalidGrid {
        /// The grid, usually its file name.
        grid: String,
        /// What's wrong with it.
        reason: String,
    },

    /// The host couldn't supply a grid's bytes.
    #[error("couldn't read {grid}: {reason}")]
    GridRead {
        /// The grid, usually its file name.
        grid: String,
        /// The host's error.
        reason: String,
    },

    /// A point outside every subgrid of a grid.
    #[error("{grid} doesn't cover latitude {latitude_deg:.6}°, longitude {longitude_deg:.6}°")]
    OutsideGrid {
        /// The grid.
        grid: String,
        /// Latitude of the point, in degrees.
        latitude_deg: f64,
        /// Longitude of the point, in degrees.
        longitude_deg: f64,
    },

    /// A point inside a grid's extent where the grid has no value, for example
    /// a geoid model offshore.
    #[error("{grid} has no value at latitude {latitude_deg:.6}°, longitude {longitude_deg:.6}°")]
    NoGridValue {
        /// The grid.
        grid: String,
        /// Latitude of the point, in degrees.
        latitude_deg: f64,
        /// Longitude of the point, in degrees.
        longitude_deg: f64,
    },

    /// The iteration that applies a grid shift in reverse didn't converge.
    #[error(
        "the inverse of {grid} didn't converge at latitude {latitude_deg:.6}°, longitude \
         {longitude_deg:.6}°, probably because the point is at the grid's edge"
    )]
    GridInverseDiverged {
        /// The grid.
        grid: String,
        /// Latitude of the point, in degrees.
        latitude_deg: f64,
        /// Longitude of the point, in degrees.
        longitude_deg: f64,
    },

    /// A height model that doesn't belong to the frame of the position.
    #[error(
        "{model} relates heights to the {expected} ellipsoid, but the position is in {frame}; \
         transform it to {expected} first"
    )]
    WrongFrameForHeightModel {
        /// The model, for example "AUSGeoid2020".
        model: String,
        /// The frame it's defined on.
        expected: FrameId,
        /// The position's frame.
        frame: FrameId,
    },

    /// A height of one kind given where another was needed.
    #[error("{operation} needs {expected}, but was given {given}")]
    WrongHeightKind {
        /// What was being done.
        operation: &'static str,
        /// The kind of height it needs.
        expected: String,
        /// The kind it was given.
        given: String,
    },
}
