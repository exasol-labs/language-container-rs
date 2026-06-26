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
mod tests {
    use super::*;

    #[test]
    fn parses_udf_object_path() {
        let src = "-- some comment\n%udf_object /tmp/my_udf.so\n-- more code";
        assert_eq!(
            parse_udf_object_path(src),
            Some(std::path::PathBuf::from("/tmp/my_udf.so"))
        );
        assert!(parse_udf_object_path("no directive here").is_none());
    }

    #[test]
    fn parses_bucketfs_path_with_trailing_semicolon() {
        let src = "%udf_object /buckets/bfsdefault/default/udfs/libudf.so;";
        assert_eq!(
            parse_udf_object_path(src),
            Some(std::path::PathBuf::from(
                "/buckets/bfsdefault/default/udfs/libudf.so"
            ))
        );
    }

    #[test]
    fn parses_debug_level_with_default() {
        // absent directive → INFO
        assert_eq!(parse_debug_level("no directive here"), tracing::Level::INFO);
        // each recognised level
        assert_eq!(
            parse_debug_level("%udf_debug_level debug"),
            tracing::Level::DEBUG
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level info"),
            tracing::Level::INFO
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level warn"),
            tracing::Level::WARN
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level error"),
            tracing::Level::ERROR
        );
    }

    #[test]
    fn parses_debug_level_trailing_semicolon() {
        assert_eq!(
            parse_debug_level("%udf_debug_level debug;"),
            tracing::Level::DEBUG
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level warn;"),
            tracing::Level::WARN
        );
    }

    #[test]
    fn parses_debug_level_unrecognised_falls_back_to_info() {
        assert_eq!(
            parse_debug_level("%udf_debug_level verbose"),
            tracing::Level::INFO
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level TRACE"),
            tracing::Level::INFO
        );
    }

    #[test]
    fn parses_debug_level_case_insensitive() {
        assert_eq!(
            parse_debug_level("%udf_debug_level DEBUG"),
            tracing::Level::DEBUG
        );
        assert_eq!(
            parse_debug_level("%udf_debug_level Warn"),
            tracing::Level::WARN
        );
    }
}
