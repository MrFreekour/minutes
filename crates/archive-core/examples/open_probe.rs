//! Why does opening fail? Reports error kinds in aggregate, never a filename.
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::collections::BTreeMap;

fn walk(dir: &Dir, depth: u32, kinds: &mut BTreeMap<String, u64>, total: &mut u64) {
    if depth > 32 {
        return;
    }
    let Ok(entries) = dir.entries() else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name();
        if meta.is_dir() {
            match dir.open_dir(&name) {
                Ok(child) => walk(&child, depth + 1, kinds, total),
                Err(e) => {
                    *kinds
                        .entry(format!("open_dir: {:?}", e.kind()))
                        .or_default() += 1
                }
            }
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        *total += 1;
        if let Err(e) = dir.open(&name) {
            *kinds.entry(format!("open: {:?}", e.kind())).or_default() += 1;
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let dir = Dir::open_ambient_dir(&path, ambient_authority()).expect("open root");
    let mut kinds = BTreeMap::new();
    let mut total = 0u64;
    walk(&dir, 0, &mut kinds, &mut total);
    println!("files seen: {total}");
    if kinds.is_empty() {
        println!("  no open failures");
    }
    for (kind, count) in &kinds {
        println!("  {kind}: {count}");
    }
}
