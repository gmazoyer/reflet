use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Error)]
pub enum AsnLoadError {
    #[error("failed to read ASN file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse ASN CSV: {0}")]
    Csv(#[from] csv::Error),
}

/// Information about an Autonomous System Number.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AsnInfo {
    pub name: String,
    pub as_domain: String,
}

/// Store of ASN-to-info mappings loaded from an ipinfo CSV database.
#[derive(Debug, Clone)]
pub struct AsnStore {
    map: Arc<HashMap<u32, AsnInfo>>,
}

/// Only the CSV columns we need from the ipinfo_lite dataset.
#[derive(Deserialize)]
struct CsvRow {
    asn: String,
    as_name: String,
    as_domain: String,
}

impl AsnStore {
    /// Create an empty store (no ASN data).
    pub fn empty() -> Self {
        Self {
            map: Arc::new(HashMap::new()),
        }
    }

    /// Load ASN info from a CSV file. Supports plain `.csv` and gzipped `.csv.gz`.
    pub fn load(path: &Path) -> Result<Self, AsnLoadError> {
        let file = std::fs::File::open(path)?;
        let is_gzipped = path.extension().is_some_and(|e| e == "gz");

        let mut map = HashMap::new();

        if is_gzipped {
            let decoder = flate2::read::GzDecoder::new(file);
            Self::parse_csv(decoder, &mut map)?;
        } else {
            Self::parse_csv(file, &mut map)?;
        }

        Ok(Self { map: Arc::new(map) })
    }

    fn parse_csv<R: Read>(reader: R, map: &mut HashMap<u32, AsnInfo>) -> Result<(), AsnLoadError> {
        let mut rdr = csv::Reader::from_reader(reader);
        for result in rdr.deserialize() {
            let row: CsvRow = result?;
            let asn_str = row.asn.strip_prefix("AS").unwrap_or(&row.asn);
            let Ok(asn) = asn_str.parse::<u32>() else {
                continue;
            };
            if asn == 0 {
                continue;
            }
            // First-seen wins (deduplicate)
            map.entry(asn).or_insert(AsnInfo {
                name: row.as_name,
                as_domain: row.as_domain,
            });
        }
        Ok(())
    }

    /// Look up info for a given ASN.
    pub fn get(&self, asn: u32) -> Option<&AsnInfo> {
        self.map.get(&asn)
    }

    /// Number of unique ASNs loaded.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Get a reference to the underlying map (Arc-wrapped for cheap cloning).
    pub fn as_map(&self) -> &Arc<HashMap<u32, AsnInfo>> {
        &self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_gzipped_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv.gz");

        let csv_data = "\
network,country,country_code,continent,continent_code,asn,as_name,as_domain
1.0.0.0/24,Australia,AU,Oceania,OC,AS13335,\"Cloudflare, Inc.\",cloudflare.com
1.0.4.0/22,Australia,AU,Oceania,OC,AS38803,Wirefreebroadband Pty Ltd,wirefreebroadband.com.au
1.0.0.0/24,Germany,DE,Europe,EU,AS13335,\"Cloudflare, Inc.\",cloudflare.com
";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(csv_data.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&path, compressed).unwrap();

        let store = AsnStore::load(&path).unwrap();
        assert_eq!(store.len(), 2); // deduplicated
        let cf = store.get(13335).unwrap();
        assert_eq!(cf.name, "Cloudflare, Inc.");
        assert_eq!(cf.as_domain, "cloudflare.com");
        assert!(store.get(99999).is_none());
    }

    #[test]
    fn load_plain_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");

        let csv_data = "\
network,country,country_code,continent,continent_code,asn,as_name,as_domain
8.8.8.0/24,United States,US,North America,NA,AS15169,Google LLC,google.com
";
        std::fs::write(&path, csv_data).unwrap();

        let store = AsnStore::load(&path).unwrap();
        assert_eq!(store.len(), 1);
        let google = store.get(15169).unwrap();
        assert_eq!(google.name, "Google LLC");
        assert_eq!(google.as_domain, "google.com");
        assert!(store.get(99999).is_none());
    }

    #[test]
    fn skip_empty_and_zero_asn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.csv");

        let csv_data = "\
network,country,country_code,continent,continent_code,asn,as_name,as_domain
1.0.0.0/24,AU,AU,OC,OC,,Unknown,unknown.com
2.0.0.0/24,AU,AU,OC,OC,AS0,Reserved,reserved.com
3.0.0.0/24,AU,AU,OC,OC,AS100,Valid AS,valid.com
";
        std::fs::write(&path, csv_data).unwrap();

        let store = AsnStore::load(&path).unwrap();
        assert_eq!(store.len(), 1);
        assert!(store.get(100).is_some());
    }

    #[test]
    fn empty_store() {
        let store = AsnStore::empty();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert!(store.get(1).is_none());
        assert!(store.as_map().is_empty());
    }
}
