use anyhow::{Context, Result};
use std::path::Path;
use walkdir::WalkDir;

use crate::types::{Probe, ProbeFile};

pub fn load_probes(probe_dir: &Path) -> Result<Vec<Probe>> {
    let mut probes = Vec::new();

    if !probe_dir.exists() {
        anyhow::bail!("probe directory does not exist: {}", probe_dir.display());
    }

    for entry in WalkDir::new(probe_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
    {
        let path = entry.path();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file: ProbeFile = serde_yaml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let probe = file.probe.into_probe(id);
        probes.push(probe);
    }

    probes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(probes)
}
