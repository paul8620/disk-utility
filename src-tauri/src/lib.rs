use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use charming::component::{Legend, Title};
use charming::element::{Emphasis, ItemStyle, Label, LabelLine, Orient, Tooltip, Trigger};
use charming::series::Pie;
use charming::Chart;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FsEntry {
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ScanResult {
    parent_path: String,
    parent_name: String,
    children: Vec<FsEntry>,
    chart_options: serde_json::Value,
    chart_label_to_path: std::collections::HashMap<String, String>,
    total_size: u64,
    scanning: bool,
    other_children: Vec<FsEntry>,
}

struct AppState {
    current_path: Mutex<Option<PathBuf>>,
    nav_stack: Mutex<Vec<PathBuf>>,
    children_cache: Mutex<std::collections::HashMap<PathBuf, Vec<FsEntry>>>,
    scanning_count: Arc<Mutex<usize>>,
}

fn list_children(path: &Path) -> Vec<FsEntry> {
    let mut children = Vec::new();
    let Ok(entries) = fs::read_dir(path) else {
        return children;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&entry_path) else {
            continue;
        };
        let name = entry_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let size = if meta.is_file() { meta.len() } else { 0 };
        children.push(FsEntry {
            path: entry_path.to_string_lossy().to_string(),
            name,
            size,
            is_dir: meta.is_dir(),
        });
    }
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    children
}

fn dir_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total = total.saturating_add(meta.len());
                    } else if meta.is_dir() {
                        stack.push(entry.path());
                    }
                }
            }
        }
    }
    total
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;
    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

struct ChartResult {
    options: serde_json::Value,
    label_to_path: std::collections::HashMap<String, String>,
    other_children: Vec<FsEntry>,
}

fn build_chart(children: &[FsEntry], parent_name: &str) -> ChartResult {
    let total: u64 = children.iter().map(|c| c.size).sum();
    if total == 0 {
        return ChartResult {
            options: serde_json::json!(null),
            label_to_path: std::collections::HashMap::new(),
            other_children: Vec::new(),
        };
    }

    let threshold = total as f64 * 0.01;
    let mut data_items: Vec<(f64, String)> = Vec::new();
    let mut label_to_path = std::collections::HashMap::new();
    let mut other_size: u64 = 0;
    let mut other_children: Vec<FsEntry> = Vec::new();

    let mut sorted = children.to_vec();
    sorted.sort_by(|a, b| b.size.cmp(&a.size));

    for child in &sorted {
        if (child.size as f64) < threshold {
            other_size += child.size;
            other_children.push(child.clone());
        } else {
            let label = format!("{} ({})", child.name, format_size(child.size));
            if child.is_dir {
                label_to_path.insert(label.clone(), child.path.clone());
            }
            data_items.push((child.size as f64, label));
        }
    }
    if other_size > 0 {
        let other_label = format!("Other ({})", format_size(other_size));
        label_to_path.insert(other_label.clone(), "__OTHER__".to_string());
        data_items.push((other_size as f64, other_label));
    }

    let pie_data: Vec<(f64, &str)> = data_items
        .iter()
        .map(|(size, name)| (*size, name.as_str()))
        .collect();

    let pie = Pie::new()
        .name("Disk Usage")
        .radius(vec!["35%", "65%"])
        .center(vec!["40%", "55%"])
        .avoid_label_overlap(true)
        .item_style(ItemStyle::new().border_radius(4).border_color("#fff").border_width(2))
        .label(Label::new().show(true))
        .label_line(LabelLine::new().show(true))
        .emphasis(
            Emphasis::new().item_style(
                ItemStyle::new()
                    .shadow_blur(10)
                    .shadow_offset_x(0)
                    .shadow_color("rgba(0, 0, 0, 0.5)"),
            ),
        )
        .data(pie_data);

    let chart = Chart::new()
        .title(
            Title::new()
                .text(format!("{} — {}", parent_name, format_size(total)))
                .left("center"),
        )
        .tooltip(Tooltip::new().trigger(Trigger::Item))
        .legend(
            Legend::new()
                .orient(Orient::Vertical)
                .left("right")
                .top("middle"),
        )
        .series(pie);

    ChartResult {
        options: serde_json::to_value(&chart).unwrap_or(serde_json::json!(null)),
        label_to_path,
        other_children,
    }
}

fn make_scan_result(
    parent_path: &Path,
    children: &[FsEntry],
    scanning: bool,
) -> ScanResult {
    let parent_name = parent_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| parent_path.to_string_lossy().to_string());

    let chart = build_chart(children, &parent_name);
    let total_size: u64 = children.iter().map(|c| c.size).sum();

    ScanResult {
        parent_path: parent_path.to_string_lossy().to_string(),
        parent_name: parent_name.clone(),
        children: children.to_vec(),
        chart_options: chart.options,
        chart_label_to_path: chart.label_to_path,
        total_size,
        scanning,
        other_children: chart.other_children,
    }
}

#[tauri::command]
fn open_folder(app_handle: tauri::AppHandle) -> Result<ScanResult, String> {
    let folder = rfd::FileDialog::new()
        .pick_folder()
        .ok_or("No folder selected")?;

    let state = app_handle.state::<AppState>();

    {
        let mut nav = state.nav_stack.lock().unwrap();
        nav.clear();
    }
    {
        let mut cache = state.children_cache.lock().unwrap();
        cache.clear();
    }
    {
        let mut current = state.current_path.lock().unwrap();
        *current = Some(folder.clone());
    }

    let children = list_children(&folder);

    {
        let mut cache = state.children_cache.lock().unwrap();
        cache.insert(folder.clone(), children.clone());
    }

    spawn_size_scans(&app_handle, &folder, &children);

    let scanning = {
        let count = state.scanning_count.lock().unwrap();
        *count > 0
    };

    Ok(make_scan_result(&folder, &children, scanning))
}

#[tauri::command]
fn navigate_into(path: String, app_handle: tauri::AppHandle) -> Result<ScanResult, String> {
    let target = PathBuf::from(&path);
    let state = app_handle.state::<AppState>();

    {
        let mut current = state.current_path.lock().unwrap();
        if let Some(ref cur) = *current {
            let mut nav = state.nav_stack.lock().unwrap();
            nav.push(cur.clone());
        }
        *current = Some(target.clone());
    }

    let children = {
        let cache = state.children_cache.lock().unwrap();
        cache.get(&target).cloned()
    };

    let children = if let Some(c) = children {
        c
    } else {
        let c = list_children(&target);
        let mut cache = state.children_cache.lock().unwrap();
        cache.insert(target.clone(), c.clone());
        spawn_size_scans(&app_handle, &target, &c);
        c
    };

    let scanning = {
        let count = state.scanning_count.lock().unwrap();
        *count > 0
    };

    Ok(make_scan_result(&target, &children, scanning))
}

#[tauri::command]
fn navigate_back(app_handle: tauri::AppHandle) -> Result<Option<ScanResult>, String> {
    let state = app_handle.state::<AppState>();

    let prev = {
        let mut nav = state.nav_stack.lock().unwrap();
        nav.pop()
    };

    let Some(prev_path) = prev else {
        return Ok(None);
    };

    {
        let mut current = state.current_path.lock().unwrap();
        *current = Some(prev_path.clone());
    }

    let children = {
        let cache = state.children_cache.lock().unwrap();
        cache.get(&prev_path).cloned().unwrap_or_default()
    };

    let scanning = {
        let count = state.scanning_count.lock().unwrap();
        *count > 0
    };

    Ok(Some(make_scan_result(&prev_path, &children, scanning)))
}

#[tauri::command]
fn delete_entry(path: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    let target = PathBuf::from(&path);
    let is_dir = target.is_dir();

    let result = if is_dir {
        fs::remove_dir_all(&target)
    } else {
        fs::remove_file(&target)
    };

    match result {
        Ok(()) => {
            let state = app_handle.state::<AppState>();
            let mut cache = state.children_cache.lock().unwrap();

            if let Some(parent) = target.parent() {
                if let Some(children) = cache.get_mut(&parent.to_path_buf()) {
                    children.retain(|c| c.path != path);
                }
            }
            cache.remove(&target);

            Ok(format!("Deleted: {}", target.display()))
        }
        Err(e) => Err(format!("Error deleting: {e}")),
    }
}

#[tauri::command]
fn clean_folder(path: String, app_handle: tauri::AppHandle) -> Result<String, String> {
    let target = PathBuf::from(&path);
    if !target.is_dir() {
        return Err(format!("Not a directory: {}", target.display()));
    }

    let entries = fs::read_dir(&target).map_err(|e| format!("Cannot read directory: {e}"))?;
    let mut errors = Vec::new();
    let mut count: usize = 0;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let result = if entry_path.is_dir() {
            fs::remove_dir_all(&entry_path)
        } else {
            fs::remove_file(&entry_path)
        };
        match result {
            Ok(()) => count += 1,
            Err(e) => errors.push(format!("{}: {e}", entry_path.display())),
        }
    }

    let state = app_handle.state::<AppState>();
    let mut cache = state.children_cache.lock().unwrap();
    cache.remove(&target);

    if let Some(parent) = target.parent() {
        if let Some(children) = cache.get_mut(&parent.to_path_buf()) {
            if let Some(entry) = children.iter_mut().find(|c| c.path == path) {
                entry.size = 0;
            }
        }
    }

    if errors.is_empty() {
        Ok(format!("Cleaned {count} items from {}", target.display()))
    } else {
        Err(format!(
            "Cleaned {count} items, but {} failed:\n{}",
            errors.len(),
            errors.join("\n")
        ))
    }
}

#[tauri::command]
fn get_current_view(app_handle: tauri::AppHandle) -> Result<Option<ScanResult>, String> {
    let state = app_handle.state::<AppState>();

    let current = {
        let cur = state.current_path.lock().unwrap();
        cur.clone()
    };

    let Some(current_path) = current else {
        return Ok(None);
    };

    let children = {
        let cache = state.children_cache.lock().unwrap();
        cache.get(&current_path).cloned().unwrap_or_default()
    };

    let scanning = {
        let count = state.scanning_count.lock().unwrap();
        *count > 0
    };

    Ok(Some(make_scan_result(&current_path, &children, scanning)))
}

#[tauri::command]
fn has_back_history(app_handle: tauri::AppHandle) -> bool {
    let state = app_handle.state::<AppState>();
    let nav = state.nav_stack.lock().unwrap();
    !nav.is_empty()
}

#[derive(Debug, Clone, Serialize)]
struct ExpandedOtherResult {
    chart_options: serde_json::Value,
    chart_label_to_path: std::collections::HashMap<String, String>,
    items: Vec<FsEntry>,
}

#[tauri::command]
fn expand_others(items: Vec<FsEntry>, parent_name: String) -> ExpandedOtherResult {
    let chart = build_chart(&items, &format!("{parent_name} — Other"));
    ExpandedOtherResult {
        chart_options: chart.options,
        chart_label_to_path: chart.label_to_path,
        items,
    }
}

fn spawn_size_scans(app_handle: &tauri::AppHandle, parent: &Path, children: &[FsEntry]) {
    let dirs: Vec<FsEntry> = children.iter().filter(|c| c.is_dir).cloned().collect();
    if dirs.is_empty() {
        return;
    }

    let state = app_handle.state::<AppState>();
    {
        let mut count = state.scanning_count.lock().unwrap();
        *count += dirs.len();
    }

    let parent = parent.to_path_buf();
    for child in dirs {
        let app_handle = app_handle.clone();
        let parent = parent.clone();
        thread::spawn(move || {
            let child_path = PathBuf::from(&child.path);
            let size = dir_size(&child_path);

            let state = app_handle.state::<AppState>();

            {
                let mut cache = state.children_cache.lock().unwrap();
                if let Some(children) = cache.get_mut(&parent) {
                    if let Some(entry) = children.iter_mut().find(|c| c.path == child.path) {
                        entry.size = size;
                    }
                }
            }

            {
                let mut count = state.scanning_count.lock().unwrap();
                *count = count.saturating_sub(1);
            }

            let _ = app_handle.emit("size-update", serde_json::json!({
                "path": child.path,
                "size": size,
            }));
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            current_path: Mutex::new(None),
            nav_stack: Mutex::new(Vec::new()),
            children_cache: Mutex::new(std::collections::HashMap::new()),
            scanning_count: Arc::new(Mutex::new(0)),
        })
        .invoke_handler(tauri::generate_handler![
            open_folder,
            navigate_into,
            navigate_back,
            delete_entry,
            clean_folder,
            get_current_view,
            has_back_history,
            expand_others,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_chart_produces_valid_echarts_json() {
        let children = vec![
            FsEntry { path: "/tmp/docs".into(), name: "docs".into(), size: 500_000_000, is_dir: true },
            FsEntry { path: "/tmp/photo.jpg".into(), name: "photo.jpg".into(), size: 200_000_000, is_dir: false },
            FsEntry { path: "/tmp/src".into(), name: "src".into(), size: 100_000_000, is_dir: true },
        ];
        let result = build_chart(&children, "tmp");
        let opts = result.options;

        assert!(opts.is_object(), "chart options should be an object");
        let series = &opts["series"];
        assert!(series.is_array(), "series should be an array");
        assert_eq!(series.as_array().unwrap().len(), 1);

        let pie = &series[0];
        assert_eq!(pie["type"], "pie");
        let data = pie["data"].as_array().unwrap();
        assert!(!data.is_empty(), "data should not be empty");

        for item in data {
            assert!(item["name"].is_string(), "each data item needs a name");
            assert!(item["value"].is_number(), "each data item needs a numeric value");
        }

        assert!(result.label_to_path.contains_key(&format!("docs ({})", format_size(500_000_000))));
        assert!(!result.label_to_path.contains_key(&format!("photo.jpg ({})", format_size(200_000_000))));
    }

    #[test]
    fn test_build_chart_empty_returns_null() {
        let children = vec![
            FsEntry { path: "/tmp/empty".into(), name: "empty".into(), size: 0, is_dir: true },
        ];
        let result = build_chart(&children, "test");
        assert!(result.options.is_null());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1_048_576), "1.00 MB");
        assert_eq!(format_size(1_073_741_824), "1.00 GB");
    }

    #[test]
    fn test_list_children_nonexistent() {
        let result = list_children(Path::new("/nonexistent_path_xyz"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_chart_title_is_array() {
        let children = vec![
            FsEntry { path: "/a".into(), name: "a".into(), size: 100, is_dir: false },
        ];
        let result = build_chart(&children, "root");
        let title = &result.options["title"];
        assert!(title.is_array(), "charming serializes title as array: {:?}", title);
    }

    #[test]
    fn test_other_grouping() {
        let mut children = vec![
            FsEntry { path: "/big".into(), name: "big".into(), size: 1_000_000, is_dir: true },
        ];
        for i in 0..10 {
            children.push(FsEntry {
                path: format!("/tiny{i}"),
                name: format!("tiny{i}"),
                size: 100,
                is_dir: false,
            });
        }
        let result = build_chart(&children, "test");

        assert!(!result.other_children.is_empty(), "should have other children");
        assert_eq!(result.other_children.len(), 10);

        let other_label = result.label_to_path.iter()
            .find(|(_, v)| v.as_str() == "__OTHER__");
        assert!(other_label.is_some(), "should have __OTHER__ sentinel in label_to_path");

        let expanded = build_chart(&result.other_children, "test — Other");
        assert!(expanded.options.is_object(), "expanded chart should be valid");
        let data = expanded.options["series"][0]["data"].as_array().unwrap();
        assert_eq!(data.len(), 10, "expanded should show all other items");
    }

    #[test]
    fn test_no_other_when_all_above_threshold() {
        let children = vec![
            FsEntry { path: "/a".into(), name: "a".into(), size: 500, is_dir: false },
            FsEntry { path: "/b".into(), name: "b".into(), size: 500, is_dir: true },
        ];
        let result = build_chart(&children, "test");
        assert!(result.other_children.is_empty());
        let has_other = result.label_to_path.values().any(|v| v == "__OTHER__");
        assert!(!has_other, "no __OTHER__ sentinel when nothing grouped");
    }
}
