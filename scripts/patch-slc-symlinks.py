#!/usr/bin/env python3
"""Post-process a raw `docker export` tarball into a gzip SLC archive.

Reads an uncompressed tar from stdin, replaces the placeholder empty-file
entries for etc/hosts and etc/resolv.conf with proper symlinks into /conf/
(matching the real shipped Python3 SLC layout), and writes a gzipped tar to
stdout.  The Exasol DB injects /conf/hosts and /conf/resolv.conf at UDF
runtime, so the symlinks pick up DB-managed name-resolution config automatically.

Usage:
    docker export <container-id> | python3 scripts/patch-slc-symlinks.py > slc.tar.gz
"""

import io
import sys
import tarfile

data = sys.stdin.buffer.read()

with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as inp:
    out_buf = io.BytesIO()
    with tarfile.open(fileobj=out_buf, mode='w:gz') as out:
        for member in inp.getmembers():
            if member.name.removeprefix('./') in ('etc/hosts', 'etc/resolv.conf'):
                continue
            f = inp.extractfile(member) if member.isfile() else None
            out.addfile(member, f)
        for name, link in [('etc/hosts', '/conf/hosts'),
                            ('etc/resolv.conf', '/conf/resolv.conf')]:
            info = tarfile.TarInfo(name)
            info.type = tarfile.SYMTYPE
            info.linkname = link
            info.mode = 0o777
            out.addfile(info)

sys.stdout.buffer.write(out_buf.getvalue())
