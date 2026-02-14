use std::fs;

pub fn get_connected_monitors() -> Vec<String> {
    let mut detected = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains('-') && !name.contains("render") {
                let status_path = format!("/sys/class/drm/{}/status", name);
                if let Ok(status) = fs::read_to_string(status_path) {
                    if status.trim() == "connected" {
                        detected.push(name);
                    }
                }
            }
        }
    }
    detected
}
