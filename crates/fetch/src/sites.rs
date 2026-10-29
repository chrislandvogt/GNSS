//! CORS sites near a mark, from Geoscience Australia's site list
//! (`gws.geodesy.ga.gov.au/corsSites`).

use serde::Deserialize;

use crate::FetchError;

/// A CORS site.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    /// Four characters, upper case.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Latitude and longitude in degrees, and ellipsoidal height in metres,
    /// approximate.
    pub position: [f64; 3],
    /// Whether its data is public. Private sites' files aren't in the open
    /// archive.
    pub public: bool,
}

impl Site {
    /// Its approximate position, Earth-centred, metres.
    pub fn ecef(&self) -> [f64; 3] {
        let [lat, lon, h] = self.position;
        ecef(lat, lon, h)
    }
}

/// Reads GA's site list (the HAL JSON of `corsSites?size=…`). Sites without
/// a position or a four-character id are left out.
pub fn parse_sites(json: &str) -> Result<Vec<Site>, FetchError> {
    #[derive(Deserialize)]
    struct Page {
        #[serde(rename = "_embedded")]
        embedded: Embedded,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Embedded {
        cors_sites: Vec<Entry>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Entry {
        four_character_id: Option<String>,
        name: Option<String>,
        approximate_position: Option<Point>,
        site_status: Option<String>,
    }
    #[derive(Deserialize)]
    struct Point {
        coordinates: Vec<f64>,
    }
    let page: Page = serde_json::from_str(json).map_err(|e| FetchError::Listing(e.to_string()))?;
    let mut sites: Vec<Site> = page
        .embedded
        .cors_sites
        .into_iter()
        .filter_map(|e| {
            let id = e.four_character_id?.to_uppercase();
            let c = e.approximate_position?.coordinates;
            let (&lat, &lon) = (c.first()?, c.get(1)?);
            let h = c.get(2).copied().unwrap_or(0.0);
            (id.len() == 4 && lat.abs() <= 90.0 && lon.abs() <= 360.0).then(|| Site {
                id,
                name: e.name.unwrap_or_default(),
                position: [lat, lon, h],
                public: e.site_status.as_deref() == Some("PUBLIC"),
            })
        })
        .collect();
    sites.sort_by(|a, b| a.id.cmp(&b.id));
    sites.dedup_by(|a, b| a.id == b.id);
    Ok(sites)
}

/// A site and how far it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Nearby<'a> {
    /// The site.
    pub site: &'a Site,
    /// Straight-line distance, metres.
    pub distance: f64,
}

/// Public sites nearest a point (ECEF, metres), nearest first, at most
/// `count` and no further than `within` metres.
pub fn nearest(sites: &[Site], point: [f64; 3], count: usize, within: f64) -> Vec<Nearby<'_>> {
    let mut near: Vec<Nearby<'_>> = sites
        .iter()
        .filter(|s| s.public)
        .map(|site| {
            let p = site.ecef();
            let d = [p[0] - point[0], p[1] - point[1], p[2] - point[2]];
            Nearby {
                site,
                distance: libm::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]),
            }
        })
        .filter(|n| n.distance <= within)
        .collect();
    near.sort_by(|a, b| {
        a.distance
            .total_cmp(&b.distance)
            .then(a.site.id.cmp(&b.site.id))
    });
    near.truncate(count);
    near
}

/// GRS80 geodetic to Earth-centred coordinates; plenty for ranking sites.
pub fn ecef(lat_deg: f64, lon_deg: f64, h: f64) -> [f64; 3] {
    let a = 6_378_137.0;
    let f = 1.0 / 298.257_222_101;
    let e2 = f * (2.0 - f);
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let (sin_lat, cos_lat) = (libm::sin(lat), libm::cos(lat));
    let n = a / libm::sqrt(1.0 - e2 * sin_lat * sin_lat);
    [
        (n + h) * cos_lat * libm::cos(lon),
        (n + h) * cos_lat * libm::sin(lon),
        (n * (1.0 - e2) + h) * sin_lat,
    ]
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    const LIST: &str = r#"{"_embedded":{"corsSites":[
      {"fourCharacterId":"SYDN","name":"Sydney","approximatePosition":{"type":"Point","coordinates":[-33.7808,151.1504,86.2]},"siteStatus":"PUBLIC"},
      {"fourCharacterId":"STR1","name":"Mount Stromlo","approximatePosition":{"type":"Point","coordinates":[-35.3155,149.0101,800.0]},"siteStatus":"PUBLIC"},
      {"fourCharacterId":"PRIV","name":"Private","approximatePosition":{"type":"Point","coordinates":[-33.78,151.15,80.0]},"siteStatus":"PRIVATE"},
      {"fourCharacterId":"NOPOS","name":"No position"},
      {"fourCharacterId":"PERT","name":"Perth","approximatePosition":{"type":"Point","coordinates":[-31.8020,115.8853,12.8]},"siteStatus":"PUBLIC"}
    ]}}"#;

    #[test]
    fn public_sites_are_ranked_by_distance() {
        let sites = parse_sites(LIST).unwrap();
        assert_eq!(sites.len(), 4);
        // A mark in Parramatta.
        let mark = ecef(-33.815, 151.003, 20.0);
        let near = nearest(&sites, mark, 2, 1e6);
        let ids: Vec<&str> = near.iter().map(|n| n.site.id.as_str()).collect();
        assert_eq!(ids, ["SYDN", "STR1"]);
        assert!(
            (near[0].distance - 14_150.0).abs() < 300.0,
            "{}",
            near[0].distance
        );
        assert!(nearest(&sites, mark, 5, 20_000.0).len() == 1);
    }
}
