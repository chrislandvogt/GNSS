//! LandXML 1.2 control points: marks as `CgPoint`s on a map grid, for CAD
//! and survey packages.
//!
//! Each point's text is `northing easting elevation`, as LandXML orders it.
//! The coordinate system is named with its EPSG code, so the reader needs no
//! guesswork; the vertical datum is named too, since LandXML has no way to
//! say whether an elevation is ellipsoidal or above a geoid otherwise.

use std::fmt::Write as _;

use crate::xml::escape;

/// A control point on the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlPoint {
    /// The mark's name.
    pub name: String,
    /// Northing, in metres.
    pub northing: f64,
    /// Easting, in metres.
    pub easting: f64,
    /// Elevation, in metres, if known.
    pub elevation: Option<f64>,
    /// A description, such as the point's uncertainty.
    pub description: Option<String>,
}

/// The grid the points are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinateSystem {
    /// Its name, such as "GDA2020 / MGA zone 55".
    pub name: String,
    /// Its EPSG code, such as 7855.
    pub epsg_code: u32,
    /// The horizontal datum, such as "GDA2020".
    pub horizontal_datum: String,
    /// What elevations are measured from, such as "GRS80 ellipsoid".
    pub vertical_datum: String,
}

/// Writes control points as LandXML 1.2. `date` (YYYY-MM-DD) and `time`
/// (HH:MM:SS) stamp the file; the caller supplies them, so the same points
/// give the same file. `application` and `version` name the writer.
pub fn write(
    points: &[ControlPoint],
    system: &CoordinateSystem,
    date: &str,
    time: &str,
    application: &str,
    version: &str,
) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:schemaLocation=\"http://www.landxml.org/schema/LandXML-1.2 \
         http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd\" \
         version=\"1.2\" date=\"{}\" time=\"{}\" readOnly=\"false\" language=\"English\">\n\
         \x20 <Units>\n\
         \x20   <Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\" \
         temperatureUnit=\"celsius\" pressureUnit=\"HPA\" angularUnit=\"decimal degrees\" \
         directionUnit=\"decimal degrees\"/>\n\
         \x20 </Units>\n\
         \x20 <CoordinateSystem name=\"{}\" epsgCode=\"{}\" horizontalDatum=\"{}\" \
         verticalDatum=\"{}\"/>\n\
         \x20 <Application name=\"{}\" version=\"{}\"/>\n\
         \x20 <CgPoints>\n",
        escape(date),
        escape(time),
        escape(&system.name),
        system.epsg_code,
        escape(&system.horizontal_datum),
        escape(&system.vertical_datum),
        escape(application),
        escape(version)
    );
    for p in points {
        let description = p
            .description
            .as_ref()
            .map(|d| format!(" desc=\"{}\"", escape(d)))
            .unwrap_or_default();
        let elevation = p.elevation.map(|e| format!(" {e:.4}")).unwrap_or_default();
        let _ = writeln!(
            out,
            "    <CgPoint name=\"{}\"{description} pntSurv=\"control\">{:.4} {:.4}{elevation}</CgPoint>",
            escape(&p.name),
            p.northing,
            p.easting
        );
    }
    out.push_str("  </CgPoints>\n</LandXML>\n");
    out
}
