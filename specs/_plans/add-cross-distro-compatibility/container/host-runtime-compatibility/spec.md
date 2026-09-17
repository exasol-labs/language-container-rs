# Feature: host-runtime-compatibility

Publishes, and verifies by running, the set of database-host Linux distributions on which a registered Rust SLC starts and executes a UDF, so the compatibility claim covers running and not only building.

## Background

`container/slc-platform-contract` governs what the SLC image itself guarantees: its glibc floor, its builder identity, its library surface and its sandbox directory skeleton. Those are properties of the image, and an image that satisfies all four still fails on some hosts. This feature covers the other half: what the database host must provide before the image can run at all.

The Exasol engine does not execute a UDF in the database process. It launches `nschroot`, which creates an unprivileged user namespace and needs `CAP_SYS_ADMIN` inside it to build the sandbox the SLC root is mounted into. That capability is granted or denied by the host kernel's security module, not by the container, not by the database, and not by anything inside the SLC. AppArmor enforces it on Debian, Ubuntu and openSUSE, and SELinux enforces it on RHEL and Fedora. A host whose module denies the capability fails every UDF, including the database's own built-in languages, and the database reports the denial to the user as `Internal error: VM crashed` with SQL state `22002`, with the real cause visible only in the host kernel audit log. This project already meets that failure on its own Ubuntu CI hosts, where the signature is `apparmor="DENIED" operation="capable" profile="unprivileged_userns" comm="nschroot" capability=21 capname="sys_admin"`.

Two consequences follow, and both are requirements rather than observations. First, the security module runs in the host kernel, so a distribution's container on a foreign-kernel host exercises that foreign host's module, not the distribution's own. Evidence that a UDF runs on a distribution is therefore only produced by a run on a host of that distribution. Second, the remedy is a host setting, so it is an operator prerequisite this project publishes rather than a defect this project fixes. Each supported family version's module and its prerequisite are recorded in one committed list, keyed by that family and version pair. The key carries the version because the prerequisite is a kernel setting one version defines and another version of the same family does not. The end-to-end run applies exactly what that list records, and the user documentation is checked against the same list.

The author-host claim in `container/slc-platform-contract` and the database-host claim here answer different questions and are verified by different evidence. Conflating them publishes a build path as if it were proof that a UDF runs, which is the reading this feature exists to prevent.

## Scenarios

### Scenario: A registered SLC executes a UDF on every supported database-host kernel

* *GIVEN* an Exasol database running on a host of a family and version the committed database-host list names, with that row's recorded host prerequisite applied
* *WHEN* the SLC is installed, registered through `SCRIPT_LANGUAGES` and a scalar Rust UDF is invoked
* *THEN* the UDF MUST return its result, rather than failing with `Internal error: VM crashed` and SQL state `22002`
* *AND* every row the list names MUST be covered by a run on a host of that family at that version, because the kernel security module that decides the outcome is the host's and not the SLC's
* *AND* a run of that distribution as a container on a host of another family MUST NOT be accepted as coverage for it, because such a run exercises the other host's kernel
* *AND* a row whose run cannot be performed MUST fail the verification rather than be skipped, so an unavailable host is visible as missing evidence instead of passing as a green claim

### Scenario: The host prerequisite is a committed record, not a step buried in the harness

* *GIVEN* the single committed database-host list, each line carrying a distribution family, a distribution version, the kernel security module that family enforces, and the host prerequisite a UDF needs on that version
* *WHEN* the end-to-end run prepares a host of that family and version
* *THEN* it MUST apply exactly the prerequisite the list records for that family and version pair, which is the row key, and no other host change, so a prerequisite discovered during a run cannot stay undocumented
* *AND* a row whose UDF run needs a host change the list does not record MUST fail, because the published prerequisite would otherwise understate what an operator has to do
* *AND* the user documentation MUST name, for every listed row, the same kernel security module and the same prerequisite, and MUST state that a version the list does not name is unverified, so the published claim cannot drift from the set that is verified
* *AND* the documentation MUST name the kernel audit signature that identifies a denial, so an operator meeting `22002 VM crashed` diagnoses the host rather than bisecting the SLC or the UDF

### Scenario: Author-host and database-host support are published as two separate claims

* *GIVEN* the committed author-host list of `container/slc-platform-contract` and the committed database-host list of this feature
* *WHEN* the user documentation states which Linux distributions this project supports
* *THEN* it MUST state the author-host claim and the database-host claim separately, naming for each which list it is checked against
* *AND* it MUST NOT present a distribution's build path as evidence that a UDF runs on a database host of that distribution, because a build path is verified by building and a run is verified by running
* *AND* a family present in one list and absent from the other MUST be published as supported for that claim only, rather than being carried across
