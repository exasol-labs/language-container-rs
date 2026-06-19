[language-container-rs](../README.md)

# Installing the language container

`scripts/install.sh` builds the Docker image, exports the container filesystem, uploads it to BucketFS, and registers the `RUST` script language — all in one command:

```bash
scripts/install.sh \
  --host localhost \
  --password exasol \
  --bfs-password <write-password>
```

The BucketFS write password for the Docker image can be read with:

```bash
docker exec exasol-db bash -c \
  "xmllint --xpath '//BucketFSService[@id=\"bfsdefault\"]/Bucket[@id=\"default\"]/WritePasswd/text()' \
  /exa/etc/EXAConf"
```

Full option reference: `scripts/install.sh --help`
