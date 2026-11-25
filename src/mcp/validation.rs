#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::str::FromStr;

use image::DynamicImage;

use crate::cli::{MaskEffect, MaskRegion, MaskWindow, Region};
use crate::config::{get_active_config, PrivacyPolicy};
use crate::mcp::base64::base64_decode;

/// Resolve a potentially non-existent target path to a canonicalized path by canonicalizing
/// its nearest existing ancestor directory and re-attaching trailing components.
fn resolve_canonical_target_path(abs_path: &Path) -> Result<PathBuf, String> {
    let mut existing_ancestor = abs_path;
    let mut trailing: Vec<&std::ffi::OsStr> = Vec::new();

    while std::fs::symlink_metadata(existing_ancestor).is_err() {
        let name = existing_ancestor.file_name().ok_or_else(|| {
            format!(
                "Cannot resolve existing ancestor directory for path '{}'",
                abs_path.display()
            )
        })?;
        trailing.push(name);
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            format!(
                "Cannot resolve existing ancestor directory for path '{}'",
                abs_path.display()
            )
        })?;
    }

    let mut canonical = existing_ancestor.canonicalize().map_err(|e| {
        format!(
            "Failed to canonicalize ancestor directory '{}': {e}",
            existing_ancestor.display()
        )
    })?;

    for comp in trailing.into_iter().rev() {
        canonical.push(comp);
    }

    Ok(canonical)
}

/// Validate an untrusted MCP output file path against extension, traversal, symlink,
/// allowed directory whitelist, and overwrite policies.
pub fn validate_mcp_output_path(
    path_str: &str,
    allowed_extensions: &[&str],
    allow_overwrite: bool,
) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err("Output path cannot be empty".to_string());
    }

    if trimmed.contains('\0') {
        return Err("Output path contains illegal null bytes".to_string());
    }

    if trimmed.starts_with(r"\\.\") || trimmed.starts_with("//./") {
        return Err(format!(
            "Device paths are not permitted in outputPath: '{trimmed}'"
        ));
    }

    let p = Path::new(trimmed);

    // Extension check
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if !allowed_extensions
        .iter()
        .any(|&allowed| allowed.eq_ignore_ascii_case(&ext))
    {
        return Err(format!(
            "Invalid file extension '.{ext}'. Allowed extensions: {}",
            allowed_extensions.join(", ")
        ));
    }

    // Path traversal check: components must not escape using ParentDir ("..")
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(format!(
                "Path traversal ('..') is not permitted in outputPath: '{trimmed}'"
            ));
        }
    }

    let abs_path = if p.is_relative() {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    } else {
        p.to_path_buf()
    };

    // Refuse overwriting or traversing through a symlink at the target path itself
    if let Ok(meta) = std::fs::symlink_metadata(&abs_path) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "Refusing to write to symbolic link target '{}'",
                trimmed
            ));
        }
        if !allow_overwrite {
            return Err(format!(
                "Target file already exists: '{}'. Set allowOverwrite=true to permit overwriting.",
                p.display()
            ));
        }
    } else if !allow_overwrite && p.exists() {
        return Err(format!(
            "Target file already exists: '{}'. Set allowOverwrite=true to permit overwriting.",
            p.display()
        ));
    }

    // Allowed directories check (from active config and RGC_ALLOWED_OUTPUT_DIRS environment variable)
    let active_cfg = get_active_config();
    let mut configured_allowed: Vec<String> = active_cfg.security.allowed_output_dirs.clone();
    if let Ok(allowed_dirs_var) = std::env::var("RGC_ALLOWED_OUTPUT_DIRS") {
        for part in allowed_dirs_var.split([',', ';']) {
            let t = part.trim();
            if !t.is_empty() {
                configured_allowed.push(t.to_string());
            }
        }
    }

    if !configured_allowed.is_empty() {
        let allowed_dirs: Vec<PathBuf> = configured_allowed
            .iter()
            .map(PathBuf::from)
            .filter_map(|pb| pb.canonicalize().ok())
            .collect();

        if allowed_dirs.is_empty() {
            return Err(format!(
                "Allowed output directories configured ({configured_allowed:?}) but none are valid or accessible; refusing file write for security."
            ));
        }

        let resolved_target = resolve_canonical_target_path(&abs_path).map_err(|_| {
            format!(
                "Target outputPath '{}' cannot be resolved within allowed directories: {:?}",
                trimmed, configured_allowed
            )
        })?;

        let is_allowed = allowed_dirs
            .iter()
            .any(|allowed| resolved_target.starts_with(allowed));

        if !is_allowed {
            return Err(format!(
                "Target outputPath '{}' is outside allowed directories: {:?}",
                trimmed, configured_allowed
            ));
        }
    }

    Ok(p.to_path_buf())
}

/// Validate and safely load an untrusted image input from a base64 string or filesystem path.
///
/// Security constraints:
/// - Base64 strings are capped at 32MB to prevent memory exhaustion DoS.
/// - File paths are strictly forbidden from path traversal (`..`) and Windows device namespaces (`\\.\`, `//./`).
/// - Decoded images are strictly protected against decompression bombs using reader limits
///   (max width/height 16384, max allocation 256MB).
pub fn load_untrusted_image_input(
    input_str: &str,
    param_name: &str,
) -> Result<DynamicImage, String> {
    let trimmed = input_str.trim();
    if trimmed.is_empty() {
        return Err(format!("Parameter '{param_name}' cannot be empty"));
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);

    let is_base64 = trimmed.starts_with("data:image/")
        || (!trimmed.contains('/')
            && !trimmed.contains('\\')
            && !trimmed.contains('.')
            && trimmed.len() > 100);

    let img = if is_base64 {
        let raw_b64 = if let Some((_, rest)) = trimmed.split_once(";base64,") {
            rest.trim()
        } else {
            trimmed
        };

        if raw_b64.len() > 32 * 1024 * 1024 {
            return Err(format!(
                "Base64 image payload in '{param_name}' exceeds maximum allowed size (32MB)"
            ));
        }

        let bytes =
            base64_decode(raw_b64).map_err(|e| format!("Invalid base64 in '{param_name}': {e}"))?;

        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes));
        reader.limits(limits);
        let reader = reader
            .with_guessed_format()
            .map_err(|e| format!("Failed to guess image format in '{param_name}': {e}"))?;
        reader
            .decode()
            .map_err(|e| format!("Failed to decode image from memory in '{param_name}': {e}"))?
    } else {
        if trimmed.starts_with(r"\\.\") || trimmed.starts_with("//./") {
            return Err(format!(
                "Device paths are not permitted in '{param_name}': '{trimmed}'"
            ));
        }

        let p = Path::new(trimmed);
        for comp in p.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(format!(
                    "Path traversal ('..') is not permitted in '{param_name}': '{trimmed}'"
                ));
            }
        }

        let mut reader = image::ImageReader::open(p)
            .map_err(|e| format!("Failed to open image from '{trimmed}' in '{param_name}': {e}"))?;
        reader.limits(limits);
        reader.decode().map_err(|e| {
            format!("Failed to decode image from '{trimmed}' in '{param_name}': {e}")
        })?
    };

    if img.width() > 16384 || img.height() > 16384 {
        return Err(format!(
            "Image dimensions ({}x{}) in '{param_name}' exceed maximum allowed size (16384x16384)",
            img.width(),
            img.height()
        ));
    }

    Ok(img)
}

/// Strictly parse an optional `u32` parameter within `[min, max]`, rejecting non-integers,
/// negative numbers, and out-of-range values without lossy `as` truncation.
pub fn parse_optional_u32_range(
    arguments: &serde_json::Value,
    key: &str,
    min: u32,
    max: u32,
) -> Result<Option<u32>, String> {
    let Some(val) = arguments.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let raw = val.as_u64().ok_or_else(|| {
        format!("Invalid {key}: must be an integer between {min} and {max} (got {val})")
    })?;
    let parsed = u32::try_from(raw)
        .ok()
        .filter(|&v| v >= min && v <= max)
        .ok_or_else(|| format!("Invalid {key}: must be between {min} and {max} (got {raw})"))?;
    Ok(Some(parsed))
}

/// Strictly parse an optional `u8` parameter within `[min, max]`, rejecting non-integers,
/// negative numbers, and out-of-range values without lossy `as` truncation.
pub fn parse_optional_u8_range(
    arguments: &serde_json::Value,
    key: &str,
    min: u8,
    max: u8,
) -> Result<Option<u8>, String> {
    let Some(val) = arguments.get(key) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let raw = val.as_u64().ok_or_else(|| {
        format!("Invalid {key}: must be an integer between {min} and {max} (got {val})")
    })?;
    let parsed = u8::try_from(raw)
        .ok()
        .filter(|&v| v >= min && v <= max)
        .ok_or_else(|| format!("Invalid {key}: must be between {min} and {max} (got {raw})"))?;
    Ok(Some(parsed))
}

/// Parse privacy redaction parameters (`redactions`, `maskWindows`, `excludeWindows`,
/// `allowMissingMask`) from MCP tool arguments and merge with active configuration policy.
pub fn parse_mcp_privacy_policy(arguments: &serde_json::Value) -> Result<PrivacyPolicy, String> {
    let mut manual_masks = Vec::new();
    if let Some(redacts_val) = arguments.get("redactions") {
        if let Some(arr) = redacts_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let mr = s
                        .parse::<MaskRegion>()
                        .map_err(|e| format!("Invalid redaction '{s}': {e}"))?;
                    manual_masks.push(mr);
                } else if let Some(obj) = item.as_object() {
                    let reg_str = obj.get("region").and_then(|v| v.as_str()).ok_or_else(|| {
                        "Invalid redaction object: missing 'region' field".to_string()
                    })?;
                    let effect = if let Some(eff_str) = obj.get("effect").and_then(|v| v.as_str()) {
                        eff_str
                            .parse::<MaskEffect>()
                            .map_err(|e| format!("Invalid redaction effect '{eff_str}': {e}"))?
                    } else {
                        MaskEffect::Transparent
                    };
                    let region = Region::from_str(reg_str)
                        .map_err(|e| format!("Invalid redaction region '{reg_str}': {e}"))?;
                    manual_masks.push(MaskRegion { region, effect });
                } else {
                    return Err(format!(
                        "Invalid redaction entry '{item}': expected string or object"
                    ));
                }
            }
        }
    }

    let mut window_masks = Vec::new();
    if let Some(win_masks_val) = arguments.get("maskWindows") {
        if let Some(arr) = win_masks_val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    let mw = s
                        .parse::<MaskWindow>()
                        .map_err(|e| format!("Invalid maskWindows entry '{s}': {e}"))?;
                    window_masks.push(mw);
                } else if let Some(obj) = item.as_object() {
                    let win_str = obj.get("window").and_then(|v| v.as_str()).ok_or_else(|| {
                        "Invalid maskWindows object: missing 'window' field".to_string()
                    })?;
                    let effect = if let Some(eff_str) = obj.get("effect").and_then(|v| v.as_str()) {
                        eff_str
                            .parse::<MaskEffect>()
                            .map_err(|e| format!("Invalid maskWindows effect '{eff_str}': {e}"))?
                    } else {
                        MaskEffect::Transparent
                    };
                    window_masks.push(MaskWindow {
                        query: win_str.to_string(),
                        effect,
                    });
                }
            }
        }
    }

    let mut exclude_windows = Vec::new();
    if let Some(arr) = arguments.get("excludeWindows").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                if !s.trim().is_empty() {
                    exclude_windows.push(s.to_string());
                }
            }
        }
    }

    let request_allow_missing = arguments.get("allowMissingMask").and_then(|v| v.as_bool());
    let cfg = get_active_config();
    Ok(PrivacyPolicy::from_config_and_request(
        &cfg.privacy,
        &window_masks,
        &exclude_windows,
        &manual_masks,
        request_allow_missing,
    ))
}

#[cfg(test)]
pub(crate) static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_optional_u32_range_rejects_truncation_and_invalid_types() {
        // 2^32 + 100 must NOT truncate to 100
        let huge = serde_json::json!({ "aroundCursor": (1u64 << 32) + 100 });
        assert!(parse_optional_u32_range(&huge, "aroundCursor", 10, 4096).is_err());

        let bad_type = serde_json::json!({ "aroundCursor": "200" });
        assert!(parse_optional_u32_range(&bad_type, "aroundCursor", 10, 4096).is_err());

        let negative = serde_json::json!({ "aroundCursor": -5 });
        assert!(parse_optional_u32_range(&negative, "aroundCursor", 10, 4096).is_err());

        let below_min = serde_json::json!({ "aroundCursor": 5 });
        assert!(parse_optional_u32_range(&below_min, "aroundCursor", 10, 4096).is_err());

        let valid = serde_json::json!({ "aroundCursor": 250 });
        assert_eq!(
            parse_optional_u32_range(&valid, "aroundCursor", 10, 4096).unwrap(),
            Some(250)
        );

        let missing = serde_json::json!({});
        assert_eq!(
            parse_optional_u32_range(&missing, "aroundCursor", 10, 4096).unwrap(),
            None
        );
    }

    #[test]
    fn test_parse_optional_u8_range_rejects_truncation() {
        // 257 must NOT truncate to 1
        let overflow = serde_json::json!({ "quality": 257 });
        assert!(parse_optional_u8_range(&overflow, "quality", 1, 100).is_err());

        let valid = serde_json::json!({ "quality": 85 });
        assert_eq!(
            parse_optional_u8_range(&valid, "quality", 1, 100).unwrap(),
            Some(85)
        );
    }

    #[test]
    fn test_validate_mcp_output_path_nonexistent_nested_subdir_inside_allowed() {
        let _guard = ENV_TEST_LOCK.lock().unwrap();
        let base = std::env::temp_dir().join(format!("rgc_val_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::env::set_var("RGC_ALLOWED_OUTPUT_DIRS", base.to_string_lossy().as_ref());

        // Non-existent nested subdir inside allowed dir should resolve via canonicalized ancestor and pass
        let nested_target = base.join("new_sub").join("deep").join("shot.png");
        let res =
            validate_mcp_output_path(nested_target.to_string_lossy().as_ref(), &["png"], true);
        assert!(
            res.is_ok(),
            "Expected nested target inside allowed dir to pass: {res:?}"
        );

        // Target outside allowed dir (even in non-existent subdir) must fail
        let outside = std::env::temp_dir()
            .join("outside_rgc_test_dir")
            .join("shot.png");
        let res_out = validate_mcp_output_path(outside.to_string_lossy().as_ref(), &["png"], true);
        assert!(res_out.is_err());

        // Device path must fail
        let res_dev = validate_mcp_output_path(r"\\.\pipe\evil.png", &["png"], true);
        assert!(res_dev.is_err());

        std::env::remove_var("RGC_ALLOWED_OUTPUT_DIRS");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_parse_mcp_privacy_policy_rejects_invalid_entries() {
        let bad = serde_json::json!({ "redactions": ["not_a_region"] });
        assert!(parse_mcp_privacy_policy(&bad).is_err());

        let valid = serde_json::json!({
            "redactions": ["10,20,30,40:blur"],
            "maskWindows": ["1Password:blackout"],
            "excludeWindows": ["SecretApp"]
        });
        let policy = parse_mcp_privacy_policy(&valid).unwrap();
        assert!(!policy.mask_regions.is_empty());
        assert!(!policy.mask_windows.is_empty());
        assert!(!policy.exclude_windows.is_empty());
    }
}
