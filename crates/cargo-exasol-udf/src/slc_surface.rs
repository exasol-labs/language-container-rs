use std::sync::LazyLock;

use crate::elf::GlibcVersion;

const GLIBC_FLOOR_TEXT: &str = include_str!("../slc-glibc-floor.txt");

const LIBRARY_SURFACE_TEXT: &str = include_str!("../slc-library-surface.txt");

/// The sonames the SLC container stages, one per line in the committed
/// `slc-library-surface.txt`.
///
/// That file is the single owner of the surface: the container's staging loop
/// and the tarball contract test read the very same lines, so a library can no
/// longer be staged without `validate` accepting it, or accepted here without
/// the container shipping it.
static ALLOWED_SONAMES: LazyLock<Vec<&str>> = LazyLock::new(|| {
    LIBRARY_SURFACE_TEXT
        .lines()
        .map(str::trim)
        .filter(|soname| !soname.is_empty())
        .collect()
});

const LOADER_SONAMES: &[&str] = &["ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1"];

const VDSO_SONAME: &str = "linux-vdso.so.1";

/// Whether a referenced glibc symbol version falls within the SLC's published floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorCompliance {
    WithinFloor,
    ExceedsFloor,
}

static GLIBC_FLOOR: LazyLock<GlibcVersion> = LazyLock::new(|| {
    GlibcVersion::parse(GLIBC_FLOOR_TEXT.trim())
        .expect("slc-glibc-floor.txt must contain a parseable dot-separated version")
});

/// The lowest glibc version the SLC container ships, parsed once from the
/// committed `slc-glibc-floor.txt`.
///
/// The CLI reads the floor only here; the tarball build separately proves
/// this number still matches the shipped `libc.so.6`.
pub fn glibc_floor() -> GlibcVersion {
    GLIBC_FLOOR.clone()
}

/// Compare a UDF artifact's referenced glibc version against the SLC's floor.
pub fn check_against_floor(referenced: &GlibcVersion) -> FloorCompliance {
    if *referenced <= glibc_floor() {
        FloorCompliance::WithinFloor
    } else {
        FloorCompliance::ExceedsFloor
    }
}

fn is_known_soname(soname: &str) -> bool {
    soname == VDSO_SONAME || LOADER_SONAMES.contains(&soname) || ALLOWED_SONAMES.contains(&soname)
}

/// From an artifact's `DT_NEEDED` sonames, return those outside the SLC's
/// verified library surface.
///
/// The dynamic loader and the kernel-injected vdso are always present at
/// runtime but are never real staged files, so both are excluded here rather
/// than left for every caller to filter out again.
pub fn unknown_sonames<'a>(needed_sonames: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    needed_sonames
        .into_iter()
        .filter(|soname| !is_known_soname(soname))
        .collect()
}

#[cfg(test)]
#[path = "slc_surface_tests.rs"]
mod tests;
