//! What an import reads, before anything is transformed or adjusted: stations,
//! with whatever coordinate and constraint the file gives them, and GNSS
//! measurements, each in its own frame and epoch.
//!
//! Readers fill a [`Survey`] with what the file says, in the file's own terms,
//! so writers can give it back. Building an adjustment from it (transforming
//! to the project's frame and epoch, applying variance scaling, choosing the
//! datum) happens later, in the `api` crate.

use std::fmt;

use gnss_datum::{CoordinateEpoch, FrameId};
use nalgebra::{DMatrix, Vector3};

/// Where something was read: a file, and a line in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// The file's name, as the caller gave it.
    pub file: String,
    /// The line, counting from 1.
    pub line: usize,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, line {}", self.file, self.line)
    }
}

/// A frame, with the coordinate epoch for a dynamic frame. For a static frame
/// the epoch is `None` or its reference epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reference {
    /// The frame.
    pub frame: FrameId,
    /// The coordinate epoch, if the file gives one.
    pub epoch: Option<CoordinateEpoch>,
}

/// What a file held.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Survey {
    /// Stations (marks), in file order.
    pub stations: Vec<Station>,
    /// GNSS measurements, in file order.
    pub measurements: Vec<Measurement>,
    /// Things the reader skipped or assumed, for the person to check.
    pub notes: Vec<Note>,
}

impl Survey {
    /// Adds another file's contents to this one.
    pub fn extend(&mut self, other: Survey) {
        self.stations.extend(other.stations);
        self.measurements.extend(other.measurements);
        self.notes.extend(other.notes);
    }
}

/// Something a reader skipped or assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// Where.
    pub origin: Origin,
    /// What, in a sentence.
    pub message: String,
}

/// A station: a mark, and what the file says about its position.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    /// The mark's name.
    pub name: String,
    /// A description, if the file has one.
    pub description: Option<String>,
    /// Its coordinate, if the file gives one.
    pub coordinate: Option<Coordinate>,
    /// Whether the coordinate holds the mark in the adjustment.
    pub constraint: Constraint,
    /// Where the station was read.
    pub origin: Origin,
}

/// Whether a station's coordinate holds the mark in the adjustment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constraint {
    /// The coordinate is only a starting value.
    Free,
    /// The mark is held at its coordinate.
    Fixed,
    /// Some components are held and some free, as DynaML's `CCF` holds
    /// latitude and longitude. The code is kept as written.
    Partial(String),
}

/// A coordinate, in the frame and at the epoch it was given in.
#[derive(Debug, Clone, PartialEq)]
pub struct Coordinate {
    /// The values.
    pub value: CoordinateValue,
    /// The frame and epoch.
    pub reference: Reference,
}

/// A coordinate's values, in the form the file gave them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CoordinateValue {
    /// Earth-centred X, Y and Z, in metres.
    Cartesian(Vector3<f64>),
    /// Latitude and longitude, in radians, and a height.
    Geographic {
        /// Latitude, in radians.
        latitude_rad: f64,
        /// Longitude, in radians.
        longitude_rad: f64,
        /// The height, in metres.
        height_m: f64,
        /// What the height is measured from.
        height: HeightType,
    },
    /// A grid coordinate on the UTM grid of the frame's ellipsoid (MGA for
    /// GDA2020 and GDA94), in the southern hemisphere.
    Grid {
        /// Easting, in metres.
        easting_m: f64,
        /// Northing, in metres.
        northing_m: f64,
        /// The zone.
        zone: u8,
        /// The height, in metres.
        height_m: f64,
        /// What the height is measured from.
        height: HeightType,
    },
}

/// What a height is measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightType {
    /// The ellipsoid.
    Ellipsoidal,
    /// A geoid or height datum, such as AHD.
    Orthometric,
}

/// Which kind of GNSS measurement, as the file called it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementKind {
    /// One baseline (DynaML type G).
    Baseline,
    /// Baselines measured together, with their correlations (DynaML type X).
    BaselineCluster,
    /// Positions measured together, with their correlations (DynaML type Y).
    PointCluster,
}

/// One three-component part of a measurement, in ECEF metres.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// A vector from one mark to another.
    Vector {
        /// The mark it starts at.
        from: String,
        /// The mark it ends at.
        to: String,
        /// The vector: `to` minus `from`.
        vector: Vector3<f64>,
    },
    /// A mark's position.
    Position {
        /// The mark.
        station: String,
        /// The position.
        position: Vector3<f64>,
    },
}

/// DynAdjust's variance scaling: the covariance is multiplied by `v`, and in
/// local axes at the first mark, by `p` north, `l` east and `h` up. A
/// measurement without scaling has all four equal to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    /// The whole matrix.
    pub v: f64,
    /// North (latitude).
    pub p: f64,
    /// East (longitude).
    pub l: f64,
    /// Up (height).
    pub h: f64,
}

impl Scaling {
    /// No scaling.
    pub const NONE: Scaling = Scaling {
        v: 1.0,
        p: 1.0,
        l: 1.0,
        h: 1.0,
    };

    /// True if it changes nothing.
    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

impl Default for Scaling {
    fn default() -> Self {
        Self::NONE
    }
}

/// A GNSS measurement: vectors or positions observed together.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// What the file called it.
    pub kind: MeasurementKind,
    /// The elements, in order.
    pub elements: Vec<Element>,
    /// The covariance of the elements' components (X, Y and Z of the first,
    /// then the next), in m², as the file gives it, before any scaling.
    pub covariance: DMatrix<f64>,
    /// The frame and epoch the measurement is in.
    pub reference: Reference,
    /// Variance scaling to apply.
    pub scaling: Scaling,
    /// True if the file marks it to be left out of the adjustment.
    pub ignored: bool,
    /// Where it came from, such as a processing run or a SINEX file, if the file says.
    pub source: Option<String>,
    /// The file's own identifier for it, if it has one.
    pub id: Option<String>,
    /// Where it was read.
    pub origin: Origin,
}

impl Measurement {
    /// A short label: "324900360 to BEEC", or "BEEC" for a position, with
    /// "and n more" for a cluster.
    pub fn label(&self) -> String {
        let first = match self.elements.first() {
            Some(Element::Vector { from, to, .. }) => format!("{from} to {to}"),
            Some(Element::Position { station, .. }) => station.clone(),
            None => String::new(),
        };
        match self.elements.len() {
            0 | 1 => first,
            n => format!("{first} and {} more", n - 1),
        }
    }
}
