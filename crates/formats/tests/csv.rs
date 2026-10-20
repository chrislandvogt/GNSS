//! Reading the generic CSV formats for vectors and control.

#![allow(clippy::unwrap_used)]

use gnss_datum::{CoordinateEpoch, FrameId, GRS80};
use gnss_formats::csv::{read_control, read_vectors};
use gnss_formats::{Constraint, CoordinateValue, Element, HeightType, MeasurementKind, Reference};
use nalgebra::Vector3;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn gda2020() -> Option<Reference> {
    Some(Reference {
        frame: FrameId::Gda2020,
        epoch: None,
    })
}

#[test]
fn reads_vectors_with_either_kind_of_covariance() {
    let text = "\
# Vectors exported from a processing package
From,To,dX (m),dY (m),dZ (m),SD_X,SD_Y,SD_Z,Corr_XY,Frame,Epoch,ID
MTHR,SNGO,-6450.1070,-4661.1080,5356.1452,0.0012,0.0007,0.0006,-0.5,ITRF2014,2025-06-15T03:00:00Z,V1
\"PM 12, west\",SNGO,1.5,2.5,3.5,0.001,0.001,0.002,,WGS 84,2025.45,
";
    let survey = read_vectors("v.csv", text, None).unwrap();
    assert_eq!(survey.measurements.len(), 2);
    let m = &survey.measurements[0];
    assert_eq!(m.id.as_deref(), Some("V1"));
    assert_eq!(m.origin.line, 3);
    assert_eq!(m.reference.frame, FrameId::Itrf2014);
    assert!(m.reference.epoch.is_some());
    let c = &m.covariance;
    assert!((c[(0, 1)] + 0.5 * 0.0012 * 0.0007).abs() < 1e-20);
    assert_eq!(c[(0, 2)], 0.0);
    let Element::Vector { from, .. } = &survey.measurements[1].elements[0] else {
        panic!();
    };
    assert_eq!(from, "PM 12, west");
    assert_eq!(survey.measurements[1].reference.frame, FrameId::Wgs84);

    let full = "from;to;dx;dy;dz;cov_xx;cov_xy;cov_xz;cov_yy;cov_yz;cov_zz;ignore\n\
                A;B;1;2;3;1e-6;-2e-7;1e-7;2e-6;3e-7;4e-6;yes\n";
    let survey = read_vectors("f.csv", full, gda2020()).unwrap();
    let m = &survey.measurements[0];
    assert!(m.ignored);
    assert_eq!(m.reference, gda2020().unwrap());
    assert_eq!(
        (
            m.covariance[(1, 0)],
            m.covariance[(2, 1)],
            m.covariance[(2, 2)]
        ),
        (-2e-7, 3e-7, 4e-6)
    );
}

#[test]
fn explains_bad_vector_files() {
    let err = |text: &str, default| {
        read_vectors("e.csv", text, default)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(err("", None), "e.csv: it's empty");
    assert_eq!(
        err("from,to,dx,dy\nA,B,1,2\n", None),
        "e.csv, line 1: there's no dx, dy and dz column"
    );
    assert!(err("from,to,dx,dy,dz\nA,B,1,2,3\n", None).contains("no covariance columns"));
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z\nA,B,1,2,3,0.001,0.001,0.001\n",
            None
        ),
        "e.csv, line 2: no frame: give a frame column, or say which frame the file is in"
    );
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z,frame\nA,B,1,2,3,0.001,0.001,0.001,ITRF2014\n",
            None
        ),
        "e.csv, line 2: ITRF2014 is a dynamic frame, so it needs an epoch: give an epoch column, \
         or say which epoch the file is at"
    );
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z,epoch\nA,B,1,2,3,0.001,0.001,0.001,2018.5\n",
            gda2020()
        ),
        "e.csv, line 2: GDA2020 is a static frame, so its epoch can only be 2020.0, not 2018.5"
    );
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z\nA,B,1,x,3,0.001,0.001,0.001\n",
            gda2020()
        ),
        "e.csv, line 2: dy isn't a number: \"x\""
    );
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z\nA,B,1,2,3,0,0.001,0.001\n",
            gda2020()
        ),
        "e.csv, line 2: the variances must be positive"
    );
    assert_eq!(
        err(
            "from,to,dx,dy,dz,sd_x,sd_y,sd_z,corr_xy\nA,B,1,2,3,1,1,1,2\n",
            gda2020()
        ),
        "e.csv, line 2: corr_xy is 2, beyond ±1"
    );
    assert_eq!(
        err("from,from,dx\n", None),
        "e.csv, line 1: the column \"from\" appears twice"
    );
}

#[test]
fn reads_control_in_every_form() {
    let text = "\
name,latitude,longitude,height,sd_e,sd_n,sd_u,description
ALIC,-23 40 12.44,133 53 07.85,603.2,0.003,0.004,0.010,Alice Springs
PM1,-33.5,151.25,25.0,,,,held
";
    let survey = read_control("c.csv", text, gda2020()).unwrap();
    assert_eq!(survey.stations.len(), 2);
    let alic = &survey.stations[0];
    assert_eq!(alic.constraint, Constraint::Free);
    assert_eq!(alic.description.as_deref(), Some("Alice Springs"));
    let CoordinateValue::Geographic {
        latitude_rad,
        height,
        ..
    } = alic.coordinate.as_ref().unwrap().value
    else {
        panic!();
    };
    assert!((latitude_rad.to_degrees() + (23.0 + 40.0 / 60.0 + 12.44 / 3600.0)).abs() < 1e-13);
    assert_eq!(height, HeightType::Ellipsoidal);
    // Weighted: a position with its covariance, from local standard deviations.
    let m = &survey.measurements[0];
    assert_eq!(m.kind, MeasurementKind::PointCluster);
    assert_eq!(m.id.as_deref(), Some("ALIC"));
    let Element::Position { position, .. } = &m.elements[0] else {
        panic!();
    };
    let at = GRS80.ecef_to_geodetic(position).unwrap();
    assert!((at.ellipsoidal_height_m() - 603.2).abs() < 1e-8);
    let r = at.enu_rotation();
    let local = r * m.covariance.fixed_view::<3, 3>(0, 0) * r.transpose();
    assert!((local[(0, 0)] - 9e-6).abs() < 1e-18);
    assert!((local[(1, 1)] - 16e-6).abs() < 1e-18);
    assert!((local[(2, 2)] - 1e-4).abs() < 1e-16);
    assert!(local[(0, 1)].abs() < 1e-18);
    // No standard deviations and no constraint: held fixed.
    assert_eq!(survey.stations[1].constraint, Constraint::Fixed);
    assert_eq!(survey.measurements.len(), 1);

    let grid = "name,easting,northing,zone,orthometric_height,constraint\n\
                BM7,340477.173,5913039.786,55,166.790,free\n";
    let survey = read_control("g.csv", grid, gda2020()).unwrap();
    assert_eq!(survey.stations[0].constraint, Constraint::Free);
    assert!(matches!(
        survey.stations[0].coordinate.as_ref().unwrap().value,
        CoordinateValue::Grid {
            zone: 55,
            height: HeightType::Orthometric,
            ..
        }
    ));

    let xyz = "name,x,y,z,sd_x,sd_y,sd_z,frame,epoch\n\
               A,-4052052.7373,4212835.9835,-2545104.5867,0.01,0.01,0.01,ITRF2014,2018.0\n";
    let survey = read_control("x.csv", xyz, None).unwrap();
    let m = &survey.measurements[0];
    assert_eq!(
        m.reference.epoch,
        Some(CoordinateEpoch::from_decimal_year(2018.0).unwrap())
    );
    assert_eq!(
        m.elements[0],
        Element::Position {
            station: "A".into(),
            position: Vector3::new(-4052052.7373, 4212835.9835, -2545104.5867),
        }
    );
}

#[test]
fn explains_bad_control_files() {
    let err = |text: &str| {
        read_control("e.csv", text, gda2020())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        err("x,y,z\n1,2,3\n"),
        "e.csv, line 1: there's no name column"
    );
    assert!(err("name,a\nA,1\n").contains("no coordinate columns"));
    assert!(err("name,latitude,longitude\nA,1,2\n").contains("no height column"));
    assert_eq!(
        err("name,latitude,longitude,height\nA,95,2,3\n"),
        "e.csv, line 2: latitude 95 is beyond the poles"
    );
    assert_eq!(
        err("name,latitude,longitude,height,constraint\nA,-30,150,3,weighted\n"),
        "e.csv, line 2: mark A is weighted control, but has no standard deviations"
    );
    assert_eq!(
        err("name,latitude,longitude,orthometric_height,sd_h,sd_v\nA,-30,150,3,0.01,0.02\n"),
        "e.csv, line 2: weighted control needs an ellipsoidal height, but this one is orthometric"
    );
    assert_eq!(
        err("name,easting,northing,zone,height\nA,1,2,61,3\n"),
        "e.csv, line 2: zone \"61\" isn't a UTM zone from 1 to 60"
    );
    assert_eq!(
        err("name,latitude,longitude,height,constraint\nA,-30,150,3,sometimes\n"),
        "e.csv, line 2: constraint should be fixed, free or weighted, not \"sometimes\""
    );
}
