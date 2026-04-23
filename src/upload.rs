// Headless upload tool for CI/CD pipelines.

use backoff::{future::retry, Error as BackoffError, ExponentialBackoff};
use clap::Parser;
use console::style;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windchill_connector::{operations, Config, WindchillClient};

#[derive(Parser)]
#[command(name = "windchill-upload")]
#[command(version)]
#[command(about = "Headless Windchill upload tool for CI/CD pipelines", long_about = None)]
struct Cli {
    /// Folder URL containing the document (alternative to --document-url)
    #[arg(long, required_unless_present = "document_url")]
    folder_url: Option<String>,

    /// Document name to search for within --folder-url
    #[arg(long, required_unless_present = "document_url")]
    document: Option<String>,

    /// Windchill web URL to the document (alternative to --folder-url and --document)
    #[arg(long, required_unless_present = "folder_url")]
    document_url: Option<String>,

    /// Base64-encoded auth token (`base64(username:password)`)
    #[arg(long)]
    auth_token: String,

    /// Checkout comment/reason
    #[arg(long)]
    checkout_comment: String,

    /// Path to file to upload
    #[arg(long)]
    filepath: PathBuf,

    /// Version identifier (passed as `versionId` to the BISSELL-specific
    /// `UpdateDocument` action). Named `--version-id` rather than `--version`
    /// to avoid shadowing clap's built-in `--version` flag.
    #[arg(long)]
    version_id: String,

    /// Path to release notes file (contents are used as the check-in comment)
    #[arg(long)]
    release_notes_path: PathBuf,

    /// Base URL for Windchill (overrides config and env). Required if not set via
    /// WINDCHILL_BASE_URL or a config file.
    #[arg(long, env = "WINDCHILL_BASE_URL")]
    baseurl: Option<String>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    println!("{}", style("Windchill Upload Tool").bold().cyan());

    if !cli.filepath.exists() {
        eprintln!(
            "{}",
            style(format!("Error: File not found: {:?}", cli.filepath)).red()
        );
        std::process::exit(1);
    }

    if !cli.release_notes_path.exists() {
        eprintln!(
            "{}",
            style(format!(
                "Error: Release notes file not found: {:?}",
                cli.release_notes_path
            ))
            .red()
        );
        std::process::exit(1);
    }

    let config = Config::load(cli.baseurl)?;

    let client = WindchillClient::new(config.base_url.clone(), cli.auth_token.clone())?;

    let release_notes_raw = std::fs::read_to_string(&cli.release_notes_path)?;
    println!("{}", style("Release Notes:").bold());
    println!("{}", release_notes_raw);
    println!();

    // Windchill rejects CheckinComment values over 4000 characters.
    // Truncate to 3800 to leave room for the suffix.
    const WINDCHILL_COMMENT_LIMIT: usize = 3800;
    let release_notes = if release_notes_raw.chars().count() > WINDCHILL_COMMENT_LIMIT {
        let notes_filename = cli.release_notes_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("release_notes.txt");
        log::warn!(
            "Release notes ({} chars) exceed {} chars; truncating for Windchill checkin comment",
            release_notes_raw.chars().count(),
            WINDCHILL_COMMENT_LIMIT
        );
        let truncated: String = release_notes_raw.chars().take(WINDCHILL_COMMENT_LIMIT).collect();
        format!(
            "{}\n\n...(truncated at {} chars — full notes in attached zip as {})\n",
            truncated, WINDCHILL_COMMENT_LIMIT, notes_filename
        )
    } else {
        release_notes_raw
    };

    tokio::time::sleep(Duration::from_millis(2500)).await;

    let doc_oid = if let Some(ref document_url) = cli.document_url {
        let doc_info = operations::resolve_document_oid(&client, document_url).map_err(|e| {
            eprintln!("{}", style(format!("Error: {}", e)).red());
            e
        })?;
        log::info!(
            "Resolved document '{}' with OID: {}",
            doc_info.name,
            doc_info.id
        );
        doc_info.id
    } else {
        let folder_url = cli.folder_url.as_ref().unwrap();
        let document = cli.document.as_ref().unwrap();

        let docs = operations::retrieve_documents_from_folder(&client, folder_url)?;
        let oid = operations::get_oid_by_name(document, &docs).map_err(|e| {
            eprintln!("{}", style(format!("Error: {}", e)).red());
            e
        })?;
        log::info!("Found document '{}' with OID: {}", document, oid);
        oid
    };

    println!("{}", style("Checking out document...").yellow());
    let updated_oid =
        match checkout_document_with_retry(&client, &doc_oid, &cli.checkout_comment).await {
            Ok(oid) => {
                println!("{}", style("Document checked out successfully").green());
                log::info!("Document checked out, new OID: {}", oid);
                oid
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    style(format!("Failed to check out document: {}", e)).red()
                );
                std::process::exit(1);
            }
        };

    tokio::time::sleep(Duration::from_secs(1)).await;

    let file_bytes = std::fs::metadata(&cli.filepath)?.len();
    // Assume a floor of 500 KB/s to Windchill; minimum 5 minutes.
    let upload_timeout = Duration::from_secs(
        ((file_bytes / 500_000) + 1).max(300)
    );
    log::info!(
        "File size: {:.1} MB — upload timeout: {}s",
        file_bytes as f64 / 1_048_576.0,
        upload_timeout.as_secs()
    );

    match upload_file_with_retry(
        &client,
        &updated_oid,
        &cli.filepath,
        &cli.version_id,
        &release_notes,
        upload_timeout,
    )
    .await
    {
        Ok(_) => {
            println!("{}", style("Upload successful!").bold().green());
            Ok(())
        }
        Err(e) => {
            eprintln!("{}", style(format!("Upload failed: {}", e)).bold().red());
            eprintln!("{}", style("Attempting to undo checkout...").yellow());

            let undo_nonce = get_nonce_with_retry(&client).await?;
            if let Err(undo_err) =
                operations::undo_check_out_document(&client, &undo_nonce, &updated_oid)
            {
                eprintln!(
                    "{}",
                    style(format!("Failed to undo checkout: {}", undo_err)).red()
                );
            } else {
                println!("{}", style("Checkout undone").yellow());
            }

            std::process::exit(1);
        }
    }
}

async fn checkout_document_with_retry(
    client: &WindchillClient,
    doc_oid: &str,
    checkout_comment: &str,
) -> anyhow::Result<String> {
    let backoff = ExponentialBackoff {
        max_elapsed_time: Some(Duration::from_secs(120)),
        initial_interval: Duration::from_secs(5),
        max_interval: Duration::from_secs(30),
        ..Default::default()
    };

    let doc_oid = doc_oid.to_string();
    let checkout_comment = checkout_comment.to_string();

    retry(backoff, || async {
        let nonce = match client.get_nonce() {
            Ok(n) => n,
            Err(e) => {
                log::warn!("Failed to get nonce for checkout, retrying: {}", e);
                return Err(BackoffError::transient(anyhow::anyhow!(
                    "Nonce error: {}",
                    e
                )));
            }
        };

        match operations::check_out_document(client, &nonce, &doc_oid, &checkout_comment) {
            Ok(result) => {
                let updated_oid = match result["ID"].as_str() {
                    Some(oid) => oid.to_string(),
                    None => {
                        return Err(BackoffError::transient(anyhow::anyhow!(
                            "No ID field in checkout response"
                        )));
                    }
                };

                if updated_oid == doc_oid {
                    log::warn!("Document is checked out by another user, will retry...");
                    return Err(BackoffError::transient(anyhow::anyhow!(
                        "Document already checked out"
                    )));
                }

                Ok(updated_oid)
            }
            Err(e) => {
                log::warn!("Checkout failed, retrying: {}", e);
                Err(BackoffError::transient(anyhow::anyhow!(
                    "Checkout error: {}",
                    e
                )))
            }
        }
    })
    .await
}

async fn get_nonce_with_retry(client: &WindchillClient) -> anyhow::Result<String> {
    let backoff = ExponentialBackoff {
        max_elapsed_time: Some(Duration::from_secs(60)),
        ..Default::default()
    };

    let result = retry(backoff, || async {
        match client.get_nonce() {
            Ok(nonce) => Ok(nonce),
            Err(e) => {
                log::warn!("Failed to get nonce, retrying: {}", e);
                Err(BackoffError::transient(e))
            }
        }
    })
    .await?;

    Ok(result)
}

async fn upload_file_with_retry(
    client: &WindchillClient,
    oid: &str,
    filepath: &Path,
    version_id: &str,
    release_notes: &str,
    upload_timeout: Duration,
) -> anyhow::Result<()> {
    // Allow up to 3 full upload attempts before giving up.
    let backoff = ExponentialBackoff {
        max_elapsed_time: Some(upload_timeout * 3),
        max_interval: Duration::from_secs(30),
        ..Default::default()
    };

    let oid = oid.to_string();
    let filepath = filepath.to_path_buf();
    let version_id = version_id.to_string();
    let release_notes = release_notes.to_string();

    retry(backoff, || async {
        let nonce = match client.get_nonce() {
            Ok(n) => n,
            Err(e) => {
                log::warn!("Failed to get nonce for upload, retrying: {}", e);
                return Err(BackoffError::transient(anyhow::anyhow!(
                    "Nonce error: {}",
                    e
                )));
            }
        };

        match operations::attach_primary_content_to_document(
            client,
            &nonce,
            &oid,
            &filepath,
            &version_id,
            &release_notes,
            upload_timeout,
        ) {
            Ok((_, details)) => match serde_json::from_str::<serde_json::Value>(&details) {
                Ok(json) => {
                    if let Some(values) = json["value"].as_array() {
                        if values.len() == 1 {
                            let return_code = values[0]["returnCode"].as_str().unwrap_or("");
                            let result = values[0]["result"].as_str().unwrap_or("");

                            if return_code == "0" && result == "Success" {
                                log::info!("File upload successful, document checked in");
                                return Ok(());
                            } else {
                                log::error!(
                                    "Upload failed: returnCode={}, result={}",
                                    return_code,
                                    result
                                );
                                return Err(BackoffError::transient(anyhow::anyhow!(
                                    "Upload validation failed"
                                )));
                            }
                        }
                    }
                    Err(BackoffError::transient(anyhow::anyhow!(
                        "Invalid response format"
                    )))
                }
                Err(e) => {
                    log::warn!("Failed to parse response, retrying: {}", e);
                    Err(BackoffError::transient(anyhow::anyhow!(
                        "JSON parse error: {}",
                        e
                    )))
                }
            },
            Err(e) => {
                log::warn!("Upload failed, retrying: {}", e);
                Err(BackoffError::transient(anyhow::anyhow!(
                    "Upload error: {}",
                    e
                )))
            }
        }
    })
    .await?;

    Ok(())
}
