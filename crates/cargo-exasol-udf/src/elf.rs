use std::fmt;
use std::path::{Path, PathBuf};

use goblin::elf::Elf;
use goblin::elf::header::{ET_DYN, et_to_str};
use goblin::elf::section_header::SHN_UNDEF;

const UDF_ENTRY_PREFIX: &str = "__exa_udf_entry_";
const GLIBC_VERSION_PREFIX: &str = "GLIBC_";

/// Everything the compatibility checks need to know about a compiled UDF
/// artifact, all derived from a single read of its dynamic section.
///
/// The three facts travel together because they come from one place in the
/// file: splitting them across separate readers would put knowledge of the
/// artifact's binary format in more than one module.
#[derive(Debug)]
pub struct SharedObject {
    /// The `<NAME>` suffix of every exported `__exa_udf_entry_<NAME>` symbol.
    pub udf_names: Vec<String>,
    /// The sonames the loader must resolve for this artifact — its `DT_NEEDED` set.
    pub needed_sonames: Vec<String>,
    /// The newest glibc symbol version the artifact references, or `None` when
    /// it references no versioned glibc symbol at all.
    pub max_glibc_version: Option<GlibcVersion>,
}

/// A glibc symbol version such as `2.41`, ordered numerically component by
/// component so `2.9` ranks below `2.34`.
///
/// The artifact's referenced version and the container's published floor are
/// both this type, so the two can never be compared by two different rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion(Vec<u32>);

impl GlibcVersion {
    /// Parse a dot-separated numeric version (`2.41`, `2.2.5`), returning
    /// `None` for anything else — glibc's own non-numeric version names
    /// (`GLIBC_PRIVATE`, `GLIBC_ABI_DT_RELR`) carry no ordering.
    pub fn parse(text: &str) -> Option<Self> {
        let components: Option<Vec<u32>> = text
            .split('.')
            .map(|component| component.parse::<u32>().ok())
            .collect();
        components.map(Self)
    }
}

impl fmt::Display for GlibcVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let components: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&components.join("."))
    }
}

/// A path that could not be turned into a [`SharedObject`].
#[derive(Debug)]
pub enum ElfError {
    /// The bytes at the path could not be read at all.
    Unreadable { path: PathBuf, reason: String },
    /// The bytes were read but are not an ELF shared object.
    NotASharedObject { path: PathBuf, reason: String },
}

impl fmt::Display for ElfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "reading '{}': {}", path.display(), reason)
            }
            Self::NotASharedObject { path, reason } => write!(
                f,
                "'{}' is not a parseable ELF shared object: {}",
                path.display(),
                reason
            ),
        }
    }
}

impl std::error::Error for ElfError {}

/// Read the artifact at `path` once and derive its UDF entry points, its
/// dynamic dependencies and its highest glibc symbol-version reference.
///
/// Callers get the derived facts and never the binary format, so no other
/// module needs an ELF reader — or a `binutils` install — of its own.
pub fn read(path: &Path) -> Result<SharedObject, ElfError> {
    let bytes = std::fs::read(path).map_err(|source| ElfError::Unreadable {
        path: path.to_path_buf(),
        reason: source.to_string(),
    })?;
    shared_object_from(&bytes).map_err(|reason| ElfError::NotASharedObject {
        path: path.to_path_buf(),
        reason,
    })
}

fn shared_object_from(bytes: &[u8]) -> Result<SharedObject, String> {
    let elf = Elf::parse(bytes).map_err(|source| source.to_string())?;

    if elf.header.e_type != ET_DYN {
        return Err(format!(
            "its ELF type is ET_{}, not ET_DYN",
            et_to_str(elf.header.e_type)
        ));
    }

    let version_names = referenced_version_names(&elf);
    Ok(SharedObject {
        udf_names: udf_entry_names(&elf),
        needed_sonames: elf
            .libraries
            .iter()
            .map(|soname| (*soname).to_string())
            .collect(),
        max_glibc_version: max_glibc_version(version_names.iter().map(String::as_str)),
    })
}

fn udf_entry_names(elf: &Elf) -> Vec<String> {
    elf.dynsyms
        .iter()
        .filter(|symbol| symbol.st_shndx != SHN_UNDEF as usize)
        .filter_map(|symbol| elf.dynstrtab.get_at(symbol.st_name))
        .filter_map(|symbol_name| symbol_name.strip_prefix(UDF_ENTRY_PREFIX))
        .map(str::to_string)
        .collect()
}

fn referenced_version_names(elf: &Elf) -> Vec<String> {
    let mut names = Vec::new();
    let Some(verneed) = elf.verneed.as_ref() else {
        return names;
    };
    for needed_file in verneed {
        for needed_version in &needed_file {
            if let Some(name) = elf.dynstrtab.get_at(needed_version.vna_name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn max_glibc_version<'a>(version_names: impl IntoIterator<Item = &'a str>) -> Option<GlibcVersion> {
    version_names
        .into_iter()
        .filter_map(|name| name.strip_prefix(GLIBC_VERSION_PREFIX))
        .filter_map(GlibcVersion::parse)
        .max()
}

#[cfg(test)]
#[path = "elf_tests.rs"]
mod tests;
