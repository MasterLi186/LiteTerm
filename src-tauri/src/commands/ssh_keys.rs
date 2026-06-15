use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SshKeyInfo {
    pub name: String,
    pub path: String,
    pub key_type: String,
    pub is_public: bool,
    pub fingerprint: String,
}

#[tauri::command]
pub async fn list_ssh_keys() -> Result<Vec<SshKeyInfo>, String> {
    let ssh_dir = dirs::home_dir()
        .ok_or("无法获取用户目录")?
        .join(".ssh");

    if !ssh_dir.exists() {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();
    let entries = std::fs::read_dir(&ssh_dir).map_err(|e| format!("读取 .ssh 目录失败: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        // Only include key files (id_* or *.pub)
        let is_public = name.ends_with(".pub");
        let is_private = name.starts_with("id_") && !is_public;

        if !is_public && !is_private {
            continue;
        }

        // Determine key type from filename
        let key_type = if name.contains("ed25519") {
            "ed25519".to_string()
        } else if name.contains("ecdsa") {
            "ecdsa".to_string()
        } else if name.contains("rsa") {
            "rsa".to_string()
        } else if name.contains("dsa") {
            "dsa".to_string()
        } else {
            "unknown".to_string()
        };

        // Get fingerprint for public keys
        let fingerprint = if is_public {
            get_fingerprint(&path).unwrap_or_default()
        } else {
            // Try to get fingerprint from corresponding .pub file
            let pub_path = path.with_extension("pub");
            if pub_path.exists() {
                get_fingerprint(&pub_path).unwrap_or_default()
            } else {
                String::new()
            }
        };

        keys.push(SshKeyInfo {
            name,
            path: path.to_string_lossy().to_string(),
            key_type,
            is_public,
            fingerprint,
        });
    }

    // Sort: public keys first, then by name
    keys.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(keys)
}

fn get_fingerprint(pub_key_path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("ssh-keygen")
        .args(["-lf", &pub_key_path.to_string_lossy()])
        .output()
        .ok()?;

    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout);
        // Format: "2048 SHA256:xxxx comment (RSA)"
        // Return the SHA256 part
        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        if parts.len() >= 2 {
            return Some(parts[1].to_string());
        }
    }
    None
}

#[tauri::command]
pub async fn generate_ssh_key(
    key_type: String,
    comment: String,
) -> Result<String, String> {
    let ssh_dir = dirs::home_dir()
        .ok_or("无法获取用户目录")?
        .join(".ssh");

    // Ensure .ssh directory exists
    std::fs::create_dir_all(&ssh_dir)
        .map_err(|e| format!("创建 .ssh 目录失败: {}", e))?;

    let key_name = format!("id_{}", key_type);
    let key_path = ssh_dir.join(&key_name);

    // Check if key already exists
    if key_path.exists() {
        return Err(format!("密钥 {} 已存在", key_name));
    }

    let output = std::process::Command::new("ssh-keygen")
        .args([
            "-t", &key_type,
            "-C", &comment,
            "-f", &key_path.to_string_lossy(),
            "-N", "",
        ])
        .output()
        .map_err(|e| format!("执行 ssh-keygen 失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh-keygen 失败: {}", stderr));
    }

    // Read and return the public key
    let pub_path = key_path.with_extension("pub");
    std::fs::read_to_string(&pub_path)
        .map_err(|e| format!("读取公钥失败: {}", e))
}

#[tauri::command]
pub async fn read_ssh_public_key(path: String) -> Result<String, String> {
    let expanded = shellexpand::tilde(&path);
    std::fs::read_to_string(expanded.as_ref())
        .map_err(|e| format!("读取公钥失败: {}", e))
}
