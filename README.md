# Snapshot Downloader

A beautiful Rust application for downloading and extracting Cosmos node snapshots and binaries.

## Features

* Resumable downloads with progress indication
* Support for multi-part snapshots (automatically concatenated)
* Automatic extraction of various archive formats
* Proper error handling and logging
* Configuration via YAML file
* Uses absolute paths for all operations
* Stores data in `~/.snapshot-downloader`

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
