//! From a built network and its adjustment back to the exchange formats'
//! terms: measurements as they were adjusted (in the network's frame and
//! epoch, with scaling applied) and marks at their adjusted positions.

use gnss_adjust::{Action, Adjustment, Datum, Network};
use gnss_formats::{
    Constraint, Coordinate, CoordinateValue, Element, Measurement, Origin, Reference, Scaling,
    Station,
};

use crate::build::Built;

/// The network's frame and epoch as a reference.
pub fn reference(network: &Network) -> Reference {
    let frame = network.frame();
    Reference {
        frame,
        epoch: (!frame.is_static()).then_some(network.epoch()),
    }
}

/// The observations as adjusted: in the network's frame and epoch, with their
/// variance scaling applied, and marked ignored where an edit excludes them.
/// Each keeps the identifier it was adjusted under.
pub fn measurements(built: &Built) -> Vec<Measurement> {
    let network = &built.network;
    let name = |mark: usize| network.marks()[mark].name.clone();
    let excluded = |id: &str| {
        built
            .edits
            .iter()
            .rev()
            .find(|e| e.observation == id)
            .is_some_and(|e| e.action == Action::Exclude)
    };
    network
        .observations()
        .iter()
        .zip(&built.sources)
        .map(|(o, source)| Measurement {
            kind: source.kind,
            elements: o
                .elements
                .iter()
                .map(|e| match *e {
                    gnss_adjust::Element::Difference {
                        from, to, vector, ..
                    } => Element::Vector {
                        from: name(from),
                        to: name(to),
                        vector,
                    },
                    gnss_adjust::Element::Position { mark, position, .. } => Element::Position {
                        station: name(mark),
                        position,
                    },
                })
                .collect(),
            covariance: o.covariance.clone(),
            reference: reference(network),
            scaling: Scaling::NONE,
            ignored: excluded(&o.id),
            source: None,
            id: Some(o.id.clone()),
            origin: Origin {
                file: source.origin.file.clone(),
                line: source.origin.line,
            },
        })
        .collect()
}

/// The marks at their adjusted positions, in the network's frame and epoch,
/// held fixed or free as they were in the adjustment.
pub fn stations(built: &Built, adjustment: &Adjustment) -> Vec<Station> {
    let fixed: &[usize] = match &built.datum {
        Datum::Constrained { fixed } => fixed,
        Datum::FixedMark(mark) => std::slice::from_ref(mark),
        Datum::Inner => &[],
    };
    built
        .network
        .marks()
        .iter()
        .zip(adjustment.marks())
        .enumerate()
        .map(|(i, (mark, adjusted))| Station {
            name: mark.name.clone(),
            description: None,
            coordinate: Some(Coordinate {
                value: CoordinateValue::Cartesian(adjusted.position),
                reference: reference(&built.network),
            }),
            constraint: if fixed.contains(&i) {
                Constraint::Fixed
            } else {
                Constraint::Free
            },
            origin: Origin {
                file: String::new(),
                line: 0,
            },
        })
        .collect()
}
