//! Sparse 64-Tree (`4x4x4` brick) baker for MagicaVoxel `.vox` unit rigs.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use anyhow::{anyhow, Result};
use ron::ser::{to_string_pretty, PrettyConfig};
use wf_core::{BakedVoxelAsset, SparseBrick4};

/// Loads a MagicaVoxel `.vox` file via `dot_vox` and packs its primary model into `SparseBrick4` bricks.
pub fn bake_vox_file(path: &Path) -> Result<BakedVoxelAsset> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("Non-UTF8 path: {}", path.display()))?;
    let vox_data = dot_vox::load(path_str)
        .map_err(|e| anyhow!("Failed to parse .vox file '{}': {}", path_str, e))?;

    let model = vox_data
        .models
        .first()
        .ok_or_else(|| anyhow!("No voxel models found in '{}'", path_str))?;

    let mut bricks_map: BTreeMap<(u8, u8, u8), [[u8; 16]; 4]> = BTreeMap::new();
    for v in &model.voxels {
        let b_pos = (v.x / 4, v.y / 4, v.z / 4);
        let lx = (v.x % 4) as usize;
        let ly = (v.y % 4) as usize;
        let lz = (v.z % 4) as usize;
        let entry = bricks_map.entry(b_pos).or_insert([[0u8; 16]; 4]);
        entry[lz][lx + ly * 4] = v.i.saturating_add(1);
    }

    let bricks: Vec<SparseBrick4> = bricks_map
        .into_iter()
        .map(|(brick_pos, voxels_z_yx)| SparseBrick4 {
            brick_pos,
            voxels_z_yx,
        })
        .collect();

    let palette_rgba = vox_data
        .palette
        .iter()
        .map(|c| (c.r, c.g, c.b, c.a))
        .collect();

    Ok(BakedVoxelAsset {
        model_name: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unit")
            .to_string(),
        dimensions: (model.size.x, model.size.y, model.size.z),
        non_empty_voxels: model.voxels.len(),
        palette_rgba,
        bricks,
    })
}

/// Bakes every `.vox` rig inside `vox_dir` into `out_vox_dir/<slug>_vox.ron`.
pub fn bake_vox_directory(vox_dir: &Path, out_vox_dir: &Path) -> Result<usize> {
    fs::create_dir_all(out_vox_dir)?;
    let pretty = PrettyConfig::new().depth_limit(3).separate_tuple_members(true);
    let mut entries: Vec<_> = fs::read_dir(vox_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("vox"))
        .collect();
    entries.sort();

    let mut count = 0usize;
    for path in entries {
        let baked = bake_vox_file(&path)?;
        let out_file = out_vox_dir.join(format!("{}_vox.ron", baked.model_name));
        fs::write(out_file, to_string_pretty(&baked, pretty.clone())?)?;
        count += 1;
    }
    Ok(count)
}
