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

/// Extract the connection name from a `%connection <name>` script directive.
///
/// Returns the first such name found, or `None` when no `%connection` is
/// present. Used as `script_name` in the MT_IMPORT credential request.
#[cfg(feature = "connect-back")]
pub fn parse_connection_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("%connection") {
            let name = rest.trim().trim_end_matches(';').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
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
}
