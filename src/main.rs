use clap::{Parser, Subcommand};
use console::style;
use std::io::{self, Write};
use std::path::PathBuf;
use windchill_connector::{auth, listing, operations, Config, WindchillClient};

#[derive(Parser)]
#[command(name = "windchill")]
#[command(version)]
#[command(about = "CLI for Windchill PLM operations", long_about = None)]
struct Cli {
    /// Base URL for Windchill (overrides config and env)
    #[arg(long, env = "WINDCHILL_BASE_URL", global = true)]
    baseurl: Option<String>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize the user config file at ~/.config/windchill/config.toml.
    /// Pass --baseurl to populate the base URL directly.
    Init,

    /// View the folder/file tree of a container
    GetTree {
        /// OID of the container to explore (e.g. `OR:wt.inf.library.WTLibrary:1234567890`)
        oid: String,
    },

    /// Retrieve document details
    GetDocument {
        /// OID of the document
        oid: String,
    },

    /// Check out a document
    CheckoutDocument {
        /// OID of the document
        oid: String,
        /// Reason for checkout
        #[arg(short, long, default_value = "Checking out document")]
        reason: String,
    },

    /// Undo document checkout
    UndoCheckout {
        /// OID of the document
        oid: String,
    },

    /// Check in a document
    CheckinDocument {
        /// OID of the document
        oid: String,
        /// Reason for checkin
        #[arg(short, long, default_value = "Checking in document")]
        reason: String,
    },

    /// Attach primary content to a document (upload + check-in).
    /// Uses a BISSELL-specific OData action — see `operations::attach_primary_content_to_document`.
    AttachPrimaryContent {
        /// OID of the document (must already be checked out)
        oid: String,
        /// Path to the file to attach
        filepath: PathBuf,
        /// Version identifier (passed to the custom `UpdateDocument` action)
        version: String,
    },

    /// Download a document and its attachments
    DownloadDocument {
        /// OID of the document
        oid: String,
        /// Output directory
        output_dir: PathBuf,
    },

    /// Resolve a Windchill web URL to its document OID
    ResolveUrl {
        /// Windchill web URL to a document
        url: String,
    },

    /// Start interactive mode
    Interactive,
}

fn main() -> anyhow::Result<()> {
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

    if let Some(Commands::Init) = &cli.command {
        let config_path = Config::create_default_config()?;
        if let Some(url) = cli.baseurl.as_ref() {
            std::fs::write(&config_path, format!("base_url = \"{}\"\n", url))?;
            println!(
                "{}",
                style(format!(
                    "Wrote config with base_url={} to {:?}",
                    url, config_path
                ))
                .green()
            );
        } else {
            println!(
                "{}",
                style(format!("Created config file at: {:?}", config_path)).green()
            );
            println!("Edit this file and set `base_url` to your Windchill server URL,");
            println!("or re-run `windchill init --baseurl <URL>` to set it directly.");
        }
        return Ok(());
    }

    let mut config = Config::load(cli.baseurl)?;

    println!("{}", style("Windchill CLI").bold().cyan());
    let (username, auth_token) = auth::prompt_for_credentials()?;
    config.username = Some(username);
    config.auth_token = Some(auth_token.clone());

    let client = WindchillClient::new(config.base_url.clone(), auth_token)?;

    match &cli.command {
        Some(Commands::GetTree { oid }) => {
            println!(
                "{}",
                style(format!("Fetching tree for OID: {}", oid)).yellow()
            );

            let tree = listing::fetch_item_tree(&client, oid)?;
            listing::print_tree(&tree, "", true);
        }

        Some(Commands::GetDocument { oid }) => {
            println!(
                "{}",
                style(format!("Retrieving document: {}", oid)).yellow()
            );

            let (doc_data, _) = listing::retrieve_document_data(&client, oid)?;
            let json: serde_json::Value = serde_json::from_str(&doc_data)?;
            println!("{}", serde_json::to_string_pretty(&json)?);
        }

        Some(Commands::CheckoutDocument { oid, reason }) => {
            println!(
                "{}",
                style(format!("Checking out document: {}", oid)).yellow()
            );

            let nonce = client.get_nonce()?;
            let result = operations::check_out_document(&client, &nonce, oid, reason)?;

            let updated_oid = result["ID"].as_str().unwrap_or("");
            if updated_oid == oid {
                println!("{}", style("Document is already checked out.").yellow());
            } else {
                println!("{}", style(format!("Updated OID: {}", updated_oid)).green());
            }
        }

        Some(Commands::UndoCheckout { oid }) => {
            println!("{}", style(format!("Undoing checkout: {}", oid)).yellow());

            let nonce = client.get_nonce()?;
            let result = operations::undo_check_out_document(&client, &nonce, oid)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        Some(Commands::CheckinDocument { oid, reason }) => {
            println!(
                "{}",
                style(format!("Checking in document: {}", oid)).yellow()
            );

            let nonce = client.get_nonce()?;
            let result = operations::check_in_document(&client, &nonce, oid, reason)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        Some(Commands::AttachPrimaryContent {
            oid,
            filepath,
            version,
        }) => {
            println!(
                "{}",
                style(format!("Attaching file to document: {}", oid)).yellow()
            );

            let nonce = client.get_nonce()?;
            let (_, details) = operations::attach_primary_content_to_document(
                &client,
                &nonce,
                oid,
                filepath,
                version,
                "File attached",
                std::time::Duration::from_secs(600),
            )?;

            let json: serde_json::Value = serde_json::from_str(&details)?;
            println!("{}", serde_json::to_string_pretty(&json)?);
        }

        Some(Commands::ResolveUrl { url }) => {
            println!("{}", style("Resolving document URL...").yellow());

            let doc_info = operations::resolve_document_oid(&client, url)?;
            println!("{} {}", style("Document:").bold(), doc_info.name);
            println!("{} {}", style("OID:").bold().green(), doc_info.id);
        }

        Some(Commands::DownloadDocument { oid, output_dir }) => {
            println!(
                "{}",
                style(format!("Downloading document: {}", oid)).yellow()
            );

            operations::download_document_with_attachments(&client, oid, output_dir)?;
            println!(
                "{}",
                style(format!(
                    "Document and attachments downloaded to {:?}",
                    output_dir
                ))
                .green()
            );
        }

        Some(Commands::Interactive) | None => {
            run_interactive(&client)?;
        }

        Some(Commands::Init) => unreachable!("Init handled above"),
    }

    Ok(())
}

fn run_interactive(client: &WindchillClient) -> anyhow::Result<()> {
    println!("{}", style("Interactive Mode").bold().cyan());
    println!("Type 'help' for available commands or 'exit' to quit");

    loop {
        print!("{}", style("windcli:> ").green().bold());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        let command = parts[0];

        match command {
            "help" => {
                println!("Available commands:");
                println!("  get_tree <oid>             - View folder/file tree");
                println!("  get_document <oid>         - Retrieve document details");
                println!("  checkout <oid> [reason]    - Check out a document");
                println!("  undo_checkout <oid>        - Undo checkout");
                println!("  checkin <oid> [reason]     - Check in a document");
                println!("  resolve_url <url>          - Resolve document URL to OID");
                println!("  download <oid> <output>    - Download document");
                println!("  exit | quit                - Exit the program");
            }
            "get_tree" => {
                if let Some(&oid) = parts.get(1) {
                    match listing::fetch_item_tree(client, oid) {
                        Ok(tree) => listing::print_tree(&tree, "", true),
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: get_tree <oid>").yellow());
                }
            }
            "get_document" => {
                if let Some(&oid) = parts.get(1) {
                    match listing::retrieve_document_data(client, oid) {
                        Ok((doc_data, _)) => {
                            let json: serde_json::Value = serde_json::from_str(&doc_data)?;
                            println!("{}", serde_json::to_string_pretty(&json)?);
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: get_document <oid>").yellow());
                }
            }
            "checkout" => {
                if let Some(&oid) = parts.get(1) {
                    let reason = parts
                        .get(2..)
                        .map(|p| p.join(" "))
                        .unwrap_or_else(|| "Checking out document".to_string());

                    match client.get_nonce() {
                        Ok(nonce) => {
                            match operations::check_out_document(client, &nonce, oid, &reason) {
                                Ok(result) => {
                                    println!("{}", serde_json::to_string_pretty(&result)?);
                                }
                                Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                            }
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: checkout <oid> [reason]").yellow());
                }
            }
            "undo_checkout" => {
                if let Some(&oid) = parts.get(1) {
                    match client.get_nonce() {
                        Ok(nonce) => {
                            match operations::undo_check_out_document(client, &nonce, oid) {
                                Ok(result) => {
                                    println!("{}", serde_json::to_string_pretty(&result)?);
                                }
                                Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                            }
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: undo_checkout <oid>").yellow());
                }
            }
            "checkin" => {
                if let Some(&oid) = parts.get(1) {
                    let reason = parts
                        .get(2..)
                        .map(|p| p.join(" "))
                        .unwrap_or_else(|| "Checking in document".to_string());

                    match client.get_nonce() {
                        Ok(nonce) => {
                            match operations::check_in_document(client, &nonce, oid, &reason) {
                                Ok(result) => {
                                    println!("{}", serde_json::to_string_pretty(&result)?);
                                }
                                Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                            }
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: checkin <oid> [reason]").yellow());
                }
            }
            "resolve_url" => {
                if let Some(&url) = parts.get(1) {
                    match operations::resolve_document_oid(client, url) {
                        Ok(doc_info) => {
                            println!("{} {}", style("Document:").bold(), doc_info.name);
                            println!("{} {}", style("OID:").bold().green(), doc_info.id);
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!(
                        "{}",
                        style("Usage: resolve_url <windchill_web_url>").yellow()
                    );
                }
            }
            "download" => {
                if parts.len() >= 3 {
                    let oid = parts[1];
                    let output = PathBuf::from(parts[2]);

                    match operations::download_document_with_attachments(client, oid, &output) {
                        Ok(_) => {
                            println!("{}", style(format!("Downloaded to {:?}", output)).green())
                        }
                        Err(e) => eprintln!("{}", style(format!("Error: {}", e)).red()),
                    }
                } else {
                    eprintln!("{}", style("Usage: download <oid> <output_dir>").yellow());
                }
            }
            "exit" | "quit" => {
                println!("{}", style("Goodbye!").cyan());
                break;
            }
            _ => {
                eprintln!("{}", style(format!("Unknown command: {}", command)).red());
                println!("Type 'help' for available commands");
            }
        }
    }

    Ok(())
}
