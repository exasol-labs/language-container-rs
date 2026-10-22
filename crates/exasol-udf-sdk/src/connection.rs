/// Credentials for a named Exasol CONNECTION object or any external system.
///
/// The field names match the proto `connection_information_rep`, so the same
/// type also reads the `connection_information` of an IMPORT or EXPORT
/// specification under either spec-parsing feature.
#[derive(Debug, Clone)]
#[cfg_attr(
    any(feature = "import", feature = "export"),
    derive(serde::Deserialize)
)]
pub struct ConnectionObject {
    pub kind: String,
    pub address: String,
    pub user: String,
    pub password: String,
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
