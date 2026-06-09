# Outlook: change-docker-alpine-base

## Runtime-downloaded ADBC drivers

The slim Alpine image opens the door to dynamically loading ADBC drivers (e.g. a MySQL ADBC driver compiled as a `.so`) that are stored in BucketFS and copied into the UDF sandbox at runtime rather than bundled in the image.

### What must be true

| Constraint | Detail |
|------------|--------|
| **glibc-linked** | The `.so` must be built against glibc (Debian builder), not musl. Pure-Go (`CGO_ENABLED=0`) or Rust cdylib for `x86_64-unknown-linux-musl` (which statically embeds musl) are the exception — they have no external libc dep. |
| **glibc version ≤ host** | Built on Debian 12 (bookworm, glibc 2.36) → version-matched to the Exasol node. |
| **Network egress** | Confirmed allowed from UDF processes. |
| **Writable path** | `/tmp` inside the Exasol container is writable. |
| **Executable path** | `/tmp` is on the overlay filesystem (not a separate `noexec` tmpfs) — dlopen works. Verify with `cat /proc/mounts` from a UDF if needed. |

### Escape hatch if `/tmp` is `noexec`

`memfd_create()` creates an anonymous in-memory fd that is always executable regardless of mount flags:

```c
int fd = memfd_create("adbc_driver", MFD_CLOEXEC);
write(fd, so_bytes, so_size);
dlopen("/proc/self/fd/<fd>", RTLD_NOW);
```

Requires the seccomp profile to allow syscall 319 (`memfd_create`). Given that outbound sockets are allowed, this is likely permitted too.

### Transitive dependencies

A downloaded `.so` may itself depend on other `.so`s (e.g. `libssl.so.3`, `libmysqlclient.so`). These must be either:
- already present on the Exasol host, or
- also downloaded and placed in a path covered by `LD_LIBRARY_PATH` or the bundled `ld.so.cache` before the primary driver is dlopen'd.

### Lifetime

The `/tmp` sandbox is torn down between UDF container restarts. A lazy-load pattern (check `/tmp/libadbc_driver_mysql.so` exists before downloading) avoids redundant transfers within a session but cannot cache across sessions.
