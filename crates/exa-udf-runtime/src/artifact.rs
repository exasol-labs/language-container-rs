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

/// Extract the cluster node IP from the ZMQ endpoint string the runtime used
/// to connect (e.g. `tcp://10.0.0.5:6583` → `Some("10.0.0.5")`).
///
/// Returns `None` for input lacking the `tcp://` scheme or a `:` port
/// separator after the host.
#[cfg(feature = "connect-back")]
pub fn parse_cluster_ip(endpoint: &str) -> Option<String> {
    let rest = endpoint.strip_prefix("tcp://")?;
    let (host, _port) = rest.rsplit_once(':')?;
    if host.is_empty() {
        return None;
    }
    Some(host.to_string())
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

    #[cfg(feature = "connect-back")]
    #[test]
    fn parse_cluster_ip_strips_scheme_and_port() {
        assert_eq!(
            parse_cluster_ip("tcp://10.0.0.5:6583"),
            Some("10.0.0.5".into())
        );
        assert_eq!(
            parse_cluster_ip("tcp://192.168.1.100:8563"),
            Some("192.168.1.100".into())
        );
        assert_eq!(parse_cluster_ip("bad"), None);
        assert_eq!(parse_cluster_ip("tcp://noport"), None);
    }
}
