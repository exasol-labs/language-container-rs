/// Extract the `.so` path from a `%udf_object <path>` script option.
///
/// Returns the first such directive found, or `None` if the source carries no
/// `%udf_object` option (the JIT path, unsupported in v1).
pub fn parse_udf_object_path(source: &str) -> Option<std::path::PathBuf> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("%udf_object") {
            let path = rest.trim().trim_end_matches(';').trim();
            if !path.is_empty() {
                return Some(std::path::PathBuf::from(path));
            }
        }
    }
    None
}

/// Extract the tracing level from a `%udf_debug_level <level>` script option.
///
/// Returns the first such directive found mapped to `tracing::Level`, or
/// `tracing::Level::INFO` when the directive is absent or the level token is
/// not recognised.  Level names are matched case-insensitively.
pub fn parse_debug_level(source: &str) -> tracing::Level {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("%udf_debug_level") {
            let token = rest.trim().trim_end_matches(';').trim();
            return match token.to_ascii_lowercase().as_str() {
                "debug" => tracing::Level::DEBUG,
                "info" => tracing::Level::INFO,
                "warn" => tracing::Level::WARN,
                "error" => tracing::Level::ERROR,
                _ => tracing::Level::INFO,
            };
        }
    }
    tracing::Level::INFO
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
