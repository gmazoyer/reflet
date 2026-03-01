use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Error)]
pub enum CommunityLoadError {
    #[error("failed to read community file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse community file {file}: {reason}")]
    Parse { file: String, reason: String },
}

/// Type of a community definition (standard or large).
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CommunityType {
    Standard,
    Large,
}

/// A wildcard community pattern (e.g. `1299:20xxx`).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommunityPattern {
    pub pattern: String,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: CommunityType,
}

/// How a single colon-separated segment of a community key should be matched.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SegmentMatcher {
    Exact { value: String },
    Range { start: u32, end: u32 },
    Wildcard { pattern: String },
}

/// A community definition whose key contains at least one range segment.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommunityRange {
    pub segments: Vec<SegmentMatcher>,
    pub description: String,
    #[serde(rename = "type")]
    pub kind: CommunityType,
}

/// All loaded community definitions.
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct CommunityDefinitions {
    pub standard: HashMap<String, String>,
    pub large: HashMap<String, String>,
    pub patterns: Vec<CommunityPattern>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<CommunityRange>,
}

/// Wrapper around community definitions that can be shared across handlers.
#[derive(Debug, Clone)]
pub struct CommunityStore {
    definitions: CommunityDefinitions,
    loaded: bool,
}

impl CommunityStore {
    /// Create an empty (no-op) store.
    pub fn empty() -> Self {
        Self {
            definitions: CommunityDefinitions::default(),
            loaded: false,
        }
    }

    /// Load community definitions from a directory of definition files.
    pub fn load(dir: &Path) -> Result<Self, CommunityLoadError> {
        let mut defs = CommunityDefinitions::default();

        // 1. Read well-known.txt first if present
        let well_known = dir.join("well-known.txt");
        if well_known.exists() {
            parse_community_file(&well_known, &mut defs)?;
        }

        // 2. Scan src_*.txt template files
        let mut template_asns: HashMap<String, Vec<u32>> = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("src_") && name.ends_with(".txt") {
                    let asns = parse_template_asns(&entry.path())?;
                    if !asns.is_empty() {
                        template_asns.insert(name, asns);
                    }
                }
            }
        }

        // Collect known template file names for pointer detection
        let template_names: Vec<String> = template_asns.keys().cloned().collect();

        // Track ASNs processed via symlink so we don't duplicate them in template expansion
        let mut symlink_asns: Vec<u32> = Vec::new();

        // 3. Scan as*.txt files — symlinks get template expansion, others parsed normally
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("as") || !name.ends_with(".txt") {
                    continue;
                }

                // Check if this is a symlink
                let meta = std::fs::symlink_metadata(entry.path())?;
                if meta.is_symlink() {
                    // Extract ASN from filename: as133086.txt -> 133086
                    if let Some(asn_str) = name
                        .strip_prefix("as")
                        .and_then(|s| s.strip_suffix(".txt"))
                        && let Ok(asn) = asn_str.parse::<u32>()
                    {
                        // Read through the symlink (std::fs::read_to_string follows symlinks)
                        let content = std::fs::read_to_string(entry.path())?;
                        let lines = extract_template_lines(&content);
                        for (key_template, desc_template) in &lines {
                            let key = key_template.replace("<ASN>", &asn.to_string());
                            let desc = desc_template.replace("<ASN>", &asn.to_string());
                            insert_community_entry(&key, &desc, &mut defs);
                        }
                        symlink_asns.push(asn);
                    }
                    continue;
                }

                // Skip if this is a pointer to a template file
                if is_pointer_file(&entry.path(), &template_names)? {
                    continue;
                }
                parse_community_file(&entry.path(), &mut defs)?;
            }
        }

        // 4. Expand #-BEGIN_ASN templates only for ASNs NOT already processed via symlink
        for (template_name, asns) in &template_asns {
            let template_path = dir.join(template_name);
            let content = std::fs::read_to_string(&template_path)?;
            let lines = extract_template_lines(&content);
            for &asn in asns {
                if symlink_asns.contains(&asn) {
                    continue;
                }
                for (key_template, desc_template) in &lines {
                    let key = key_template.replace("<ASN>", &asn.to_string());
                    let desc = desc_template.replace("<ASN>", &asn.to_string());
                    insert_community_entry(&key, &desc, &mut defs);
                }
            }
        }

        Ok(Self {
            definitions: defs,
            loaded: true,
        })
    }

    /// Whether community definitions were loaded (vs empty).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Get a reference to the loaded definitions.
    pub fn definitions(&self) -> &CommunityDefinitions {
        &self.definitions
    }
}

/// Parse a community definition file, adding entries to the given definitions.
fn parse_community_file(
    path: &Path,
    defs: &mut CommunityDefinitions,
) -> Result<(), CommunityLoadError> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, desc)) = line.split_once(',') {
            let key = key.trim();
            let desc = desc.trim();
            if key.is_empty() || desc.is_empty() {
                continue;
            }
            insert_community_entry(key, desc, defs);
        }
    }
    Ok(())
}

/// Check if a segment string represents a numeric range (e.g. `13001-13099`).
fn segment_is_range(seg: &str) -> bool {
    if let Some((left, right)) = seg.split_once('-') {
        !left.is_empty()
            && !right.is_empty()
            && left.bytes().all(|b| b.is_ascii_digit())
            && right.bytes().all(|b| b.is_ascii_digit())
    } else {
        false
    }
}

/// Check if a segment contains wildcard characters (`x` or `n`).
fn segment_has_wildcard(seg: &str) -> bool {
    seg.contains('x') || seg.contains('n')
}

/// Build a `SegmentMatcher` for a single colon-separated segment.
fn build_segment_matcher(seg: &str) -> SegmentMatcher {
    if segment_is_range(seg) {
        let (left, right) = seg.split_once('-').unwrap();
        SegmentMatcher::Range {
            start: left.parse().unwrap(),
            end: right.parse().unwrap(),
        }
    } else if segment_has_wildcard(seg) {
        SegmentMatcher::Wildcard {
            pattern: seg.to_string(),
        }
    } else {
        SegmentMatcher::Exact {
            value: seg.to_string(),
        }
    }
}

/// Insert a community key/description into the appropriate map, patterns, or ranges list.
fn insert_community_entry(key: &str, desc: &str, defs: &mut CommunityDefinitions) {
    let colon_count = key.chars().filter(|&c| c == ':').count();

    let kind = if colon_count >= 2 {
        CommunityType::Large
    } else {
        CommunityType::Standard
    };

    let segments: Vec<&str> = key.split(':').collect();
    let has_range = segments.iter().any(|s| segment_is_range(s));
    let has_wildcard = segments.iter().any(|s| segment_has_wildcard(s));

    if has_range {
        let matchers = segments.iter().map(|s| build_segment_matcher(s)).collect();
        defs.ranges.push(CommunityRange {
            segments: matchers,
            description: desc.to_string(),
            kind,
        });
    } else if has_wildcard {
        defs.patterns.push(CommunityPattern {
            pattern: key.to_string(),
            description: desc.to_string(),
            kind,
        });
    } else {
        match kind {
            CommunityType::Standard => {
                defs.standard.insert(key.to_string(), desc.to_string());
            }
            CommunityType::Large => {
                defs.large.insert(key.to_string(), desc.to_string());
            }
        }
    }
}

/// Extract ASNs from `#-BEGIN_ASN` / `#-END_ASN` blocks in a template file.
fn parse_template_asns(path: &Path) -> Result<Vec<u32>, CommunityLoadError> {
    let content = std::fs::read_to_string(path)?;
    let mut asns = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "#-BEGIN_ASN" {
            in_block = true;
            continue;
        }
        if trimmed == "#-END_ASN" {
            in_block = false;
            continue;
        }
        if in_block {
            let trimmed = trimmed.trim_start_matches('#').trim();
            if let Ok(asn) = trimmed.parse::<u32>() {
                asns.push(asn);
            }
        }
    }

    Ok(asns)
}

/// Extract template community lines (outside `#-BEGIN_ASN`/`#-END_ASN` blocks).
fn extract_template_lines(content: &str) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "#-BEGIN_ASN" {
            in_block = true;
            continue;
        }
        if trimmed == "#-END_ASN" {
            in_block = false;
            continue;
        }
        if in_block || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, desc)) = trimmed.split_once(',') {
            let key = key.trim();
            let desc = desc.trim();
            if !key.is_empty() && !desc.is_empty() {
                lines.push((key.to_string(), desc.to_string()));
            }
        }
    }

    lines
}

/// Check if a file is a pointer to a template (first non-comment non-empty line is `src_*.txt`).
fn is_pointer_file(path: &Path, template_names: &[String]) -> Result<bool, CommunityLoadError> {
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Ok(template_names.contains(&trimmed.to_string()));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn parse_standard_community() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as1299.txt"),
            "1299:20500,Amsterdam (Peer)\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(
            store.definitions().standard.get("1299:20500"),
            Some(&"Amsterdam (Peer)".to_string())
        );
    }

    #[test]
    fn parse_large_community() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as6695.txt"),
            "6695:1914:150,Continent: Europe\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(
            store.definitions().large.get("6695:1914:150"),
            Some(&"Continent: Europe".to_string())
        );
    }

    #[test]
    fn parse_wildcard_pattern() {
        let dir = temp_dir();
        fs::write(dir.path().join("as1299.txt"), "1299:20xxx,EU Peers\n").unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert!(store.definitions().standard.get("1299:20xxx").is_none());
        assert_eq!(store.definitions().patterns.len(), 1);
        assert_eq!(store.definitions().patterns[0].pattern, "1299:20xxx");
        assert_eq!(store.definitions().patterns[0].description, "EU Peers");
    }

    #[test]
    fn parse_well_known() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("well-known.txt"),
            "65535:666,Blackhole\n65535:65281,NO_EXPORT\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(
            store.definitions().standard.get("65535:666"),
            Some(&"Blackhole".to_string())
        );
        assert_eq!(
            store.definitions().standard.get("65535:65281"),
            Some(&"NO_EXPORT".to_string())
        );
    }

    #[test]
    fn parse_template_expansion() {
        let dir = temp_dir();
        let template = "\
# Template for DE-CIX
<ASN>:1914:100,Location: DE-CIX
#-BEGIN_ASN
# 6695
6695
#-END_ASN
";
        fs::write(dir.path().join("src_decix.txt"), template).unwrap();
        // Pointer file
        fs::write(dir.path().join("as6695.txt"), "src_decix.txt\n").unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(
            store.definitions().large.get("6695:1914:100"),
            Some(&"Location: DE-CIX".to_string())
        );
        // Pointer file should not add raw entries
        assert!(store.definitions().standard.get("src_decix.txt").is_none());
    }

    #[test]
    fn skip_comments_and_empty_lines() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as100.txt"),
            "# This is a comment\n\n100:1,Valid entry\n# Another comment\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(store.definitions().standard.len(), 1);
        assert_eq!(
            store.definitions().standard.get("100:1"),
            Some(&"Valid entry".to_string())
        );
    }

    #[test]
    fn pointer_file_is_skipped() {
        let dir = temp_dir();
        let template = "\
<ASN>:100:1,Test
#-BEGIN_ASN
200
#-END_ASN
";
        fs::write(dir.path().join("src_test.txt"), template).unwrap();
        fs::write(dir.path().join("as200.txt"), "src_test.txt\n").unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        // Template expansion should work
        assert_eq!(
            store.definitions().large.get("200:100:1"),
            Some(&"Test".to_string())
        );
        // No garbage entries from the pointer file
        assert_eq!(store.definitions().standard.len(), 0);
    }

    #[test]
    fn empty_store() {
        let store = CommunityStore::empty();
        assert!(!store.is_loaded());
        assert!(store.definitions().standard.is_empty());
        assert!(store.definitions().large.is_empty());
        assert!(store.definitions().patterns.is_empty());
    }

    #[test]
    fn loaded_store_reports_loaded() {
        let dir = temp_dir();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert!(store.is_loaded());
    }

    #[test]
    fn wildcard_with_n() {
        let dir = temp_dir();
        fs::write(dir.path().join("as300.txt"), "300:1nnnn,Some pattern\n").unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(store.definitions().patterns.len(), 1);
        assert_eq!(store.definitions().patterns[0].pattern, "300:1nnnn");
    }

    #[test]
    fn parse_range_in_value() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as15169.txt"),
            "15169:13001-13099,Some range description\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        // Should NOT be in exact maps
        assert!(store.definitions().standard.get("15169:13001-13099").is_none());
        // Should be in ranges
        assert_eq!(store.definitions().ranges.len(), 1);
        let r = &store.definitions().ranges[0];
        assert_eq!(r.description, "Some range description");
        assert_eq!(r.segments.len(), 2);
        assert!(matches!(&r.segments[0], SegmentMatcher::Exact { value } if value == "15169"));
        assert!(matches!(&r.segments[1], SegmentMatcher::Range { start: 13001, end: 13099 }));
    }

    #[test]
    fn parse_range_in_asn() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as65001.txt"),
            "65001-65004:6509,Range in ASN\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(store.definitions().ranges.len(), 1);
        let r = &store.definitions().ranges[0];
        assert!(matches!(&r.segments[0], SegmentMatcher::Range { start: 65001, end: 65004 }));
        assert!(matches!(&r.segments[1], SegmentMatcher::Exact { value } if value == "6509"));
    }

    #[test]
    fn parse_range_with_wildcard() {
        let dir = temp_dir();
        fs::write(
            dir.path().join("as65511.txt"),
            "65511-65513:nnn,Mixed range and wildcard\n",
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        // Range takes precedence: goes to ranges, not patterns
        assert_eq!(store.definitions().ranges.len(), 1);
        assert!(store.definitions().patterns.is_empty());
        let r = &store.definitions().ranges[0];
        assert!(matches!(&r.segments[0], SegmentMatcher::Range { start: 65511, end: 65513 }));
        assert!(matches!(&r.segments[1], SegmentMatcher::Wildcard { pattern } if pattern == "nnn"));
    }

    #[cfg(unix)]
    #[test]
    fn parse_symlink_template() {
        let dir = temp_dir();
        let template = "\
# Template for test
<ASN>:100:1,Test for AS <ASN>
";
        fs::write(dir.path().join("src_test.txt"), template).unwrap();
        // Create symlink: as99999.txt -> src_test.txt
        std::os::unix::fs::symlink(
            dir.path().join("src_test.txt"),
            dir.path().join("as99999.txt"),
        )
        .unwrap();
        let store = CommunityStore::load(dir.path()).unwrap();
        assert_eq!(
            store.definitions().large.get("99999:100:1"),
            Some(&"Test for AS 99999".to_string())
        );
    }

    #[test]
    fn empty_store_has_no_ranges() {
        let store = CommunityStore::empty();
        assert!(store.definitions().ranges.is_empty());
    }
}
