# Snapshot Downloader

A beautiful Rust application for downloading and extracting Cosmos node snapshots and binaries.

## Features

* Resumable downloads with progress indication
* Support for multi-part snapshots (automatically concatenated)
* Automatic extraction of various archive formats
* Proper error handling and logging
* Configuration via YAML file
* Built-in profiles (resolve snapshot + binary from a live index)
* Uses absolute paths for all operations
* Stores data in `~/.snapshot-downloader`

## Quick start with profiles

Built-in profiles fetch the **latest published snapshot** for a chain from a
`snapshots.json` index (see [Snapshot index format](#snapshot-index-format-snapshotsjson) below) — no
`config.yaml` required. Profiles resolve the matching binary URL from the snapshot
`version`, download both, apply curated node config, and start the node.

```bash
# Mainnet snapshot, leveldb / pruned
snapshot-downloader --profile cronos-mainnet-leveldb-pruned

# Another network / db / pruning combination
snapshot-downloader --profile cronos-pos-mainnet-versiondb-pruned

# Testnet variant
snapshot-downloader --profile cronos-testnet-rocksdb-pruned

# List every available profile name
snapshot-downloader --list-profiles
```

Profile names follow `<base>-<db>-<pruning>`:

| Part | Values |
|------|--------|
| base | `cronos-mainnet`, `cronos-testnet`, `cronos-pos-mainnet`, `cronos-pos-testnet` |
| db | `leveldb`, `rocksdb`, `versiondb`, `versiondb-memiavl`, … |
| pruning | `pruned`, `default`, `archive`, `none`, … |

> **Profiles file lookup.** The profile definitions are read from (in order):
> the `--profiles <path>` flag, `./profiles.yaml`, a `profiles.yaml` next to
> the executable, then a copy embedded into the binary at build time — so the
> tool works even when run outside the source tree.
>
> **Binary assets are platform-specific.** `binary_url` templates expand
> `{os}`/`{arch}` (e.g. `Linux`/`arm64`); profile binary URLs are typically
> published for Linux, so profile runs are intended for Linux hosts.

Current profiles available via `--list-profiles`:

```text
cronos-mainnet
  cronos-mainnet-leveldb-archive
  cronos-mainnet-leveldb-default
  cronos-mainnet-leveldb-pruned
  cronos-mainnet-rocksdb-archive
  cronos-mainnet-rocksdb-default
  cronos-mainnet-rocksdb-pruned
  cronos-mainnet-versiondb-archive
  cronos-mainnet-versiondb-pruned
  cronos-mainnet-versiondb-memiavl-none

cronos-testnet
  cronos-testnet-leveldb-archive
  cronos-testnet-leveldb-default
  cronos-testnet-leveldb-pruned
  cronos-testnet-rocksdb-archive
  cronos-testnet-rocksdb-default
  cronos-testnet-rocksdb-pruned
  cronos-testnet-versiondb-archive
  cronos-testnet-versiondb-default
  cronos-testnet-versiondb-pruned
  cronos-testnet-versiondb-memiavl-none

cronos-pos-mainnet
  cronos-pos-mainnet-leveldb-archive
  cronos-pos-mainnet-leveldb-default
  cronos-pos-mainnet-leveldb-pruned
  cronos-pos-mainnet-rocksdb-archive
  cronos-pos-mainnet-rocksdb-default
  cronos-pos-mainnet-rocksdb-pruned
  cronos-pos-mainnet-versiondb-pruned

cronos-pos-testnet
  cronos-pos-testnet-leveldb-pruned
  cronos-pos-testnet-rocksdb-pruned
  cronos-pos-testnet-versiondb-pruned
```

| Flag | Default | Description |
|------|---------|-------------|
| `--profile <name>` | — | a full `<base>-<db>-<pruning>` profile name (see `--list-profiles`) |
| `--config <path>` | `config.yaml` | config file used when `--profile` is not set |
| `--profiles <path>` | — | profiles file to read (overrides the cwd / embedded lookup) |
| `--list-profiles` | — | query the live index and print every available profile name |

The snapshot is written to `~/.snapshot-downloader/workspace/home/`.

## Snapshot index format (`snapshots.json`)

Profiles point at a URL that returns a JSON snapshot index. Any provider can
publish a compatible file at an HTTPS endpoint (for example
`https://example.com/snapshots.json`).

### Top-level structure

```json
{
  "generated_at": "2026-06-22T10:00:00Z",
  "chains": { ... }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `generated_at` | string (ISO 8601) | When the index was generated (informational) |
| `chains` | object | Map of chain keys to environment trees |

### Hierarchy

Snapshots are organised as a four-level path under `chains`:

```
chains
└── <chain_prefix>           e.g. "my-chain"
    └── <env_key>            e.g. "mainnet-snapshot", "testnet-snapshot"
        └── <db_type>        e.g. "leveldb", "rocksdb", "versiondb"
            └── <pruning_type>   e.g. "pruned", "default", "archive", "none"
                └── [ <snapshot entries> ]
```

A profile in `profiles.yaml` selects one leaf via `chain_prefix`, `env_key`,
`db_type`, and `pruning_type`, then downloads the **latest** entry in that
array (sorted by `last_modified` descending, with `filename` as a tiebreak).

### Snapshot entry (single-file)

```json
{
  "filename": "my-chain-pruned-20260622.tar.lz4",
  "size_bytes": 23396341266,
  "last_modified": "2026-06-22T09:42:15Z",
  "download_url": "https://snapshots.example.com/my-chain/mainnet-snapshot/rocksdb/pruned/my-chain-pruned-20260622.tar.lz4",
  "version": "v1.0.0",
  "sha256": "0d184b84cddee213b0d473b54486791c552de2c6a54a3cd5e72382fc3036610d",
  "verified": true
}
```

| Field | Required | Used by tool | Description |
|-------|----------|--------------|-------------|
| `filename` | yes | yes | Archive filename (also used to pick the latest entry) |
| `download_url` | yes* | yes | Direct download URL for single-file snapshots |
| `version` | recommended | yes | Chain binary version (e.g. `v1.7.7`); expands `{version}` / `{version_no_v}` in profile `binary_url` templates |
| `sha256` | no | yes (verified) | Published checksum for the full archive; single-file snapshots are verified after download |
| `size_bytes` | no | no | Uncompressed or archive size in bytes (informational) |
| `last_modified` | no | yes | Publish time (ISO-8601 UTC); used to pick the latest entry |
| `verified` | no | no | Whether the publisher marked the entry verified |
| `part_files` | no | yes | Present for multi-part snapshots (see below) |

\* When `part_files` is non-empty, `download_url` on the parent entry is ignored
and each part is downloaded from `part_files[].download_url` instead.

### Snapshot entry (multi-part)

Large archives may be split into ordered parts. The parent entry keeps
metadata; parts hold the actual download URLs:

```json
{
  "filename": "my-chain-archive-20260612.tar.lz4",
  "size_bytes": 7915190787761,
  "last_modified": "2026-06-13T12:15:22Z",
  "download_url": "https://snapshots.example.com/my-chain/mainnet-snapshot/leveldb/archive/my-chain-archive-20260612.tar.lz4",
  "version": "v1.0.0",
  "sha256": "",
  "verified": false,
  "part_files": [
    {
      "filename": "my-chain-archive-20260612.tar.lz4.part001",
      "size_bytes": 4294967296000,
      "download_url": "https://snapshots.example.com/my-chain/mainnet-snapshot/leveldb/archive/my-chain-archive-20260612.tar.lz4.part001",
      "sha256": "0b519b5659a431506deded69e1f172b3022b006b714488adf9e9c4527244759c"
    },
    {
      "filename": "my-chain-archive-20260612.tar.lz4.part002",
      "size_bytes": 3620223491761,
      "download_url": "https://snapshots.example.com/my-chain/mainnet-snapshot/leveldb/archive/my-chain-archive-20260612.tar.lz4.part002",
      "sha256": "77abe9dbb1c9230b8093a42171425e2903cdb06ea6c0435e31f8babd400ab6c7"
    }
  ]
}
```

| `part_files[]` field | Required | Used by tool | Description |
|----------------------|----------|--------------|-------------|
| `filename` | yes | yes | Part filename (parts are concatenated in filename order) |
| `download_url` | yes | yes | Direct download URL for this part |
| `sha256` | no | yes (verified) | Per-part checksum; verified during download when present |
| `size_bytes` | no | no | Part size in bytes (informational) |

### Minimal compatible example

The smallest index a custom provider needs for a single snapshot:

```json
{
  "chains": {
    "my-chain": {
      "mainnet-snapshot": {
        "rocksdb": {
          "pruned": [
            {
              "filename": "my-chain-pruned-20260101.tar.lz4",
              "download_url": "https://example.com/my-chain-pruned-20260101.tar.lz4",
              "version": "v1.0.0",
              "sha256": "abc123..."
            }
          ]
        }
      }
    }
  }
}
```

Point a profile at it with:

```yaml
config:
  chain_id: my-chain-1
  binary_url: "https://example.com/my-chain_{version_no_v}_{os}_{arch}.tar.gz"
  binary_relative_path: bin/my-chaind
snapshot:
  index_url: "https://example.com/snapshots.json"
  chain_prefix: my-chain
  env_key: mainnet-snapshot
  db_type: rocksdb
  pruning_type: pruned
```

## Requirements

* Rust 1.60 or later
* Cargo package manager

## Installation

```bash
# Clone the repository
git clone https://github.com/yourusername/snapshot-downloader.git
cd snapshot-downloader

# Build the application
cargo build --release
```

## Configuration

Edit the `config.yaml` file to set your snapshot and binary URLs, chain ID, and moniker:

```yaml
# URL for the snapshot to download (for single file snapshots)
snapshot_url: "https://example.com/cosmos-snapshot.tar.gz"

# URLs for multi-part snapshots (alternative to snapshot_url)
# If snapshot_urls is provided, it will be used instead of snapshot_url
# snapshot_urls:
#   - "https://example.com/cosmos-snapshot.part001.tar.gz"
#   - "https://example.com/cosmos-snapshot.part002.tar.gz"
#   - "https://example.com/cosmos-snapshot.part003.tar.gz"

# URL for the binary to download
binary_url: "https://example.com/cosmos-binary.tar.gz"

# Chain ID for the Cosmos network
chain_id: "cosmoshub-4"

# Moniker (node name) to use when initializing
moniker: "my-cosmos-node"
```

## Usage

```bash
# Run the application
cargo run --release
```

## Directory Structure

The application creates the following directory structure:

```
~/.snapshot-downloader/
├── downloads/         # Downloaded snapshot and binary files
└── workspace/
    ├── bin/           # Extracted binary files
    └── home/          # Home directory for the Cosmos node
```

## Process

1. Download the Cosmos binary
2. Extract the binary to `~/.snapshot-downloader/workspace/bin/`
3. Initialize the binary with the specified chain ID and moniker
4. Download the snapshot (single file or multi-part)
5. Extract the snapshot to `~/.snapshot-downloader/workspace/home/`

## Multi-Part Snapshots

Some snapshots are split into multiple parts for easier downloading. The application supports this by:

1. Downloading each part individually with progress indication
2. Concatenating all parts into a single file
3. Cleaning up the individual part files after concatenation

To use multi-part snapshots, configure the `snapshot_urls` array in your `config.yaml` instead of `snapshot_url`:

```yaml
snapshot_urls:
  - "https://example.com/cosmos-snapshot.part001.tar.gz"
  - "https://example.com/cosmos-snapshot.part002.tar.gz"
  - "https://example.com/cosmos-snapshot.part003.tar.gz"
```

The application will automatically detect the number of parts and handle the concatenation process.

## Error Handling

The application includes comprehensive error handling for:
- Failed downloads
- Resuming interrupted downloads
- Extraction failures
- Binary initialization issues

## License

MIT
