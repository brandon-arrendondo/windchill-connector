# windchill-connector

A Rust CLI and headless uploader for [PTC Windchill](https://www.ptc.com/en/products/windchill)
PLM operations, built on top of Windchill's OData REST API.

Ships two binaries:

- `windchill` — an interactive / scriptable CLI for exploring containers,
  retrieving documents, checking in/out, downloading, and resolving Windchill
  web URLs to canonical OIDs.
- `windchill-upload` — a headless uploader designed for CI/CD: takes a file
  plus a document reference and performs checkout → upload → check-in with
  exponential-backoff retries and automatic rollback on failure.

## Features

- Interactive CLI with colored output, spinners, and progress bars
- Headless upload mode for CI/CD pipelines with automatic retry + checkout rollback
- Resolve Windchill web URLs (the ones you copy from a browser) to document OIDs
- Download documents with all attachments
- Config via CLI args, `WINDCHILL_*` environment variables, or `~/.config/windchill/config.toml`
- Single static binary — no Python, no virtualenvs

## Install

### Prebuilt binaries

Each tagged release publishes:

- `windchill-connector-linux-x86_64.tar.gz` — both binaries
- `windchill-connector-windows-x86_64.zip` — both binaries
- `windchill-x86_64.AppImage` (Linux) — interactive CLI
- `windchill-upload-x86_64.AppImage` (Linux) — headless uploader
- `*.deb` (Debian / Ubuntu)

Grab the latest from the [Releases page](https://github.com/brandon-arrendondo/windchill-connector/releases).

### From source

```bash
git clone https://github.com/brandon-arrendondo/windchill-connector.git
cd windchill-connector
cargo build --release
# Binaries land at target/release/windchill and target/release/windchill-upload
```

### Requirements

- Rust 1.74+ (stable)

## Configuration

The base URL must be provided. In priority order:

1. CLI flag: `--baseurl https://windchill.example.com/Windchill`
2. Environment variable: `WINDCHILL_BASE_URL`
3. Config file: `~/.config/windchill/config.toml`

Create the config file interactively:

```bash
# Scaffold with a placeholder you then edit:
windchill init

# Or set the URL in one shot:
windchill --baseurl https://windchill.example.com/Windchill init
```

The config file is simply:

```toml
base_url = "https://windchill.example.com/Windchill"
```

## Usage

### Interactive mode

```bash
windchill --baseurl https://windchill.example.com/Windchill
```

You'll be prompted for credentials, then dropped into a REPL:

```
windcli:> help
windcli:> get_tree OR:wt.inf.library.WTLibrary:1234567890
windcli:> get_document OR:wt.doc.WTDocument:1352219857
windcli:> checkout OR:wt.doc.WTDocument:1352219857 Working on updates
windcli:> checkin  OR:wt.doc.WTDocument:1352219857 Completed changes
windcli:> resolve_url 'https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?oid=VR%3Awt.doc.WTDocument%3A2034364227&u8=1'
windcli:> download  OR:wt.doc.WTDocument:1352219857 ./output
windcli:> exit
```

### Direct commands

```bash
windchill get-tree OR:wt.inf.library.WTLibrary:1234567890
windchill get-document OR:wt.doc.WTDocument:1352219857
windchill checkout-document OR:wt.doc.WTDocument:1352219857 --reason "Working on updates"
windchill undo-checkout OR:wt.doc.WTDocument:1352219857
windchill checkin-document OR:wt.doc.WTDocument:1352219857 --reason "Completed changes"
windchill resolve-url 'https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?oid=VR%3Awt.doc.WTDocument%3A2034364227&u8=1'
windchill download-document OR:wt.doc.WTDocument:1352219857 ./output
```

### Headless upload (CI/CD)

Two ways to identify the target document:

**Option A — Windchill web URL (recommended):**

```bash
windchill-upload \
  --document-url "https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?ContainerOid=OR%3Awt.inf.library.WTLibrary%3A1087756099&oid=VR%3Awt.doc.WTDocument%3A2034364227&u8=1" \
  --auth-token "$(echo -n 'user:pass' | base64)" \
  --checkout-comment "CI upload" \
  --filepath ./artifact.zip \
  --version-id "1.2.3" \
  --release-notes-path ./RELEASE_NOTES.md
```

**Option B — folder URL + document name:**

```bash
windchill-upload \
  --folder-url "https://windchill.example.com/Windchill/servlet/odata/v5/DataAdmin/Containers('OR:wt.inf.library.WTLibrary:1087756099')/Folders('OR:wt.folder.SubFolder:1149016974')/Contents" \
  --document "MyProduct" \
  --auth-token "$(echo -n 'user:pass' | base64)" \
  --checkout-comment "CI upload" \
  --filepath ./artifact.zip \
  --version-id "1.2.3" \
  --release-notes-path ./RELEASE_NOTES.md
```

Example GitHub Actions step:

```yaml
- name: Upload to Windchill
  env:
    WINDCHILL_BASE_URL: ${{ vars.WINDCHILL_BASE_URL }}
    WINDCHILL_AUTH: ${{ secrets.WINDCHILL_AUTH_TOKEN }}
    WINDCHILL_DOC_URL: ${{ vars.WINDCHILL_DOCUMENT_URL }}
  run: |
    windchill-upload \
      --document-url "$WINDCHILL_DOC_URL" \
      --auth-token "$WINDCHILL_AUTH" \
      --checkout-comment "Release ${{ github.ref_name }}" \
      --filepath ./dist/product.zip \
      --version-id "${{ github.ref_name }}" \
      --release-notes-path ./RELEASE_NOTES.md
```

## A note on `attach-primary-content` / the uploader's check-in step

`windchill-connector` drives the **standard** Windchill OData API for nearly
everything (tree listing, checkout/checkin, download, URL resolution). The
**one exception** is the primary-content upload path used by both
`windchill attach-primary-content` and `windchill-upload`.

That path ends with a POST to a custom OData action:

```
/servlet/odata/v1/BissellWRS/UpdateDocument
```

This action is a **BISSELL-specific server-side extension** that performs a
check-in while also stamping the iteration with a `versionId` and a SHA-256
hash of the uploaded content. It is **not** part of stock Windchill — other
deployments will not have this endpoint and the upload call will fail.

If you are adapting `windchill-connector` to a different Windchill site,
replace the `UpdateDocument` call inside
[`src/operations.rs::attach_primary_content_to_document`](src/operations.rs)
with a standard `PTC.DocMgmt.CheckIn` action (the `check_in_document` helper
in the same file already does this for you) and drop the `--version-id` flag
from the CLI, or wire it into your own custom action.

Everything else in this crate is vendor-neutral.

## Development

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

### Pre-commit hooks

```bash
pipx install pre-commit   # or pip / brew
pre-commit install
```

Hooks run `cargo fmt`, `cargo check`, and `cargo clippy` on every commit.

## License

MIT — see [LICENSE](LICENSE).
