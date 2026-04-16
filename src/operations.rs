use crate::client::WindchillClient;
use crate::error::{Result, WindchillError};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct DocumentInfo {
    #[serde(rename = "ID")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct DocumentsResponse {
    pub value: Vec<DocumentInfo>,
}

pub fn check_out_document(
    client: &WindchillClient,
    nonce: &str,
    document_oid: &str,
    reason: &str,
) -> Result<Value> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_message("Checking out document...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let path = format!(
        "/servlet/odata/DocMgmt/Documents('{}')/PTC.DocMgmt.CheckOut",
        document_oid
    );

    let payload = json!({
        "CheckOutNote": reason
    });

    let response = client.post(&path, &payload, Some(nonce))?;
    let json: Value = response.json()?;

    spinner.finish_with_message("Document checked out");

    Ok(json)
}

pub fn undo_check_out_document(
    client: &WindchillClient,
    nonce: &str,
    document_oid: &str,
) -> Result<Value> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_message("Undoing checkout...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let path = format!(
        "/servlet/odata/DocMgmt/Documents('{}')/PTC.DocMgmt.UndoCheckOut",
        document_oid
    );

    let payload = json!({});
    let response = client.post(&path, &payload, Some(nonce))?;
    let json: Value = response.json()?;

    spinner.finish_with_message("Checkout undone");

    Ok(json)
}

pub fn check_in_document(
    client: &WindchillClient,
    nonce: &str,
    document_oid: &str,
    reason: &str,
) -> Result<Value> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_message("Checking in document...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let path = format!(
        "/servlet/odata/DocMgmt/Documents('{}')/PTC.DocMgmt.CheckIn",
        document_oid
    );

    let payload = json!({
        "CheckInNote": reason
    });

    let response = client.post(&path, &payload, Some(nonce))?;
    let json: Value = response.json()?;

    spinner.finish_with_message("Document checked in");

    Ok(json)
}

/// Upload a file as the primary content of a checked-out document, set a custom
/// version + SHA-256 via a site-specific OData action, and check it in.
///
/// Flow:
/// 1. PUT multipart content to `/v5/DocMgmt/Documents('{oid}')/PrimaryContent` (standard Windchill).
/// 2. POST to `/v1/BissellWRS/UpdateDocument` with `versionId`, `shaID`, `documentID`,
///    and `CheckinComment`. This action also performs the check-in.
///
/// NOTE: Step 2 is a **BISSELL-specific custom OData action** provided by the
/// `BissellWRS` service on our Windchill server. It is **not** part of standard
/// Windchill — other deployments will not have this endpoint and this function
/// will fail against them. If you are adapting this tool for a different
/// Windchill site, replace the `UpdateDocument` call with a standard
/// `PTC.DocMgmt.CheckIn` (see [`check_in_document`]) and move any version /
/// hash metadata into the check-in note or your own custom action.
pub fn attach_primary_content_to_document(
    client: &WindchillClient,
    nonce: &str,
    document_oid: &str,
    file_path: &Path,
    version: &str,
    checkin_comment: &str,
) -> Result<(String, String)> {
    let pb = ProgressBar::new(2);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("=>-"),
    );

    pb.set_message("Uploading file...");

    let upload_path = format!(
        "/servlet/odata/v5/DocMgmt/Documents('{}')/PrimaryContent",
        document_oid
    );

    let upload_response = client.put_file(&upload_path, file_path, nonce)?;
    let upload_result = upload_response.text()?;

    pb.inc(1);
    pb.set_message("Updating document metadata...");

    let mut file = File::open(file_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    hasher.update(&buffer);
    let sha_hash = format!("{:x}", hasher.finalize());

    // BISSELL-specific custom OData action. See doc comment above.
    let update_path = "/servlet/odata/v1/BissellWRS/UpdateDocument";
    let doc_id = document_oid.replace("OR:", "");

    let payload = json!({
        "versionId": version,
        "shaID": sha_hash,
        "documentID": doc_id,
        "CheckinComment": checkin_comment
    });

    let details_response = client.post(update_path, &payload, Some(nonce))?;
    let details_result = details_response.text()?;

    let parsed: Value = serde_json::from_str(&details_result)?;
    if let Some(values) = parsed["value"].as_array() {
        if values.len() == 1 {
            let return_code = values[0]["returnCode"].as_str().unwrap_or("");
            let result = values[0]["result"].as_str().unwrap_or("");

            if return_code != "0" || result != "Success" {
                pb.finish_with_message("Upload failed");
                return Err(WindchillError::UploadError(format!(
                    "Upload failed: returnCode={}, result={}",
                    return_code, result
                )));
            }
        }
    }

    pb.inc(1);
    pb.finish_with_message("File uploaded and document checked in");

    Ok((upload_result, details_result))
}

pub fn download_document_with_attachments(
    client: &WindchillClient,
    document_oid: &str,
    output_dir: &Path,
) -> Result<()> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_message("Downloading document metadata...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    std::fs::create_dir_all(output_dir)?;

    let doc_path = format!("/servlet/odata/DocMgmt/Documents('{}')", document_oid);
    let doc_response = client.get(&doc_path)?;
    let doc_data = doc_response.text()?;

    let doc_file_path = output_dir.join(format!("{}.json", document_oid));
    let mut doc_file = File::create(&doc_file_path)?;
    doc_file.write_all(doc_data.as_bytes())?;

    spinner.set_message("Downloading primary content...");

    let pc_path = format!(
        "/servlet/odata/DocMgmt/Documents('{}')/PrimaryContent",
        document_oid
    );
    let pc_response = client.get(&pc_path)?;
    let pc_data = pc_response.text()?;
    let pc_json: Value = serde_json::from_str(&pc_data)?;

    let pc_file_path = output_dir.join(format!("{}_primarycontent.json", document_oid));
    let mut pc_file = File::create(&pc_file_path)?;
    pc_file.write_all(pc_data.as_bytes())?;

    if let Some(content_url) = pc_json["Content"]["URL"].as_str() {
        let filename = pc_json["FileName"].as_str().unwrap_or("download");
        spinner.set_message(format!("Downloading {}...", filename));

        let file_response = reqwest::blocking::get(content_url)?;
        let file_path = output_dir.join(filename);
        let mut file = File::create(&file_path)?;
        file.write_all(&file_response.bytes()?)?;
    }

    spinner.set_message("Downloading attachments...");

    let attachments_path = format!(
        "/servlet/odata/DocMgmt/Documents('{}')/Attachments",
        document_oid
    );
    let attachments_response = client.get(&attachments_path)?;
    let attachments_data = attachments_response.text()?;
    let attachments_json: Value = serde_json::from_str(&attachments_data)?;

    if let Some(attachments) = attachments_json["value"].as_array() {
        for attachment in attachments {
            if let (Some(filename), Some(url)) = (
                attachment["FileName"].as_str(),
                attachment["Content"]["URL"].as_str(),
            ) {
                spinner.set_message(format!("Downloading attachment {}...", filename));
                let att_response = reqwest::blocking::get(url)?;
                let att_path = output_dir.join(filename);
                let mut att_file = File::create(&att_path)?;
                att_file.write_all(&att_response.bytes()?)?;
            }
        }
    }

    spinner.finish_with_message("Download complete");

    Ok(())
}

pub fn retrieve_documents_from_folder(
    client: &WindchillClient,
    folder_url: &str,
) -> Result<DocumentsResponse> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_message("Retrieving documents list...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let path = if folder_url.starts_with("http") {
        folder_url
            .split_once(client.base_url())
            .map(|(_, p)| p)
            .unwrap_or(folder_url)
    } else {
        folder_url
    };

    let response = client.get(path)?;
    let docs: DocumentsResponse = response.json()?;

    spinner.finish_with_message(format!("Found {} documents", docs.value.len()));

    Ok(docs)
}

pub fn get_oid_by_name(name: &str, documents: &DocumentsResponse) -> Result<String> {
    let matches: Vec<&DocumentInfo> = documents
        .value
        .iter()
        .filter(|doc| doc.name.contains(name))
        .collect();

    match matches.len() {
        0 => Err(WindchillError::DocumentNotFound(name.to_string())),
        1 => Ok(matches[0].id.clone()),
        _ => {
            log::warn!(
                "Multiple matches found for '{}', returning last in list",
                name
            );
            Ok(matches.last().unwrap().id.clone())
        }
    }
}

#[derive(Debug)]
pub struct WindchillUrlParams {
    pub oid: String,
    pub container_oid: Option<String>,
}

fn percent_decode(input: &str) -> String {
    let input_bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut i = 0;
    while i < input_bytes.len() {
        if input_bytes[i] == b'%' && i + 2 < input_bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                output.push(byte as char);
                i += 3;
                continue;
            }
        }
        output.push(input_bytes[i] as char);
        i += 1;
    }
    output
}

/// Parse a Windchill web URL to extract the document OID and container OID.
///
/// Expected URL format:
/// `https://<host>/Windchill/app/#ptc1/tcomp/infoPage?ContainerOid=OR%3A...&oid=VR%3A...&u8=1`
pub fn parse_document_url(url: &str) -> Result<WindchillUrlParams> {
    let fragment = url
        .split_once('#')
        .map(|(_, f)| f)
        .ok_or_else(|| WindchillError::Other("URL has no fragment section".into()))?;

    let query_str = fragment
        .split_once('?')
        .map(|(_, q)| q)
        .ok_or_else(|| WindchillError::Other("URL fragment has no query parameters".into()))?;

    let params: HashMap<&str, String> = query_str
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k, percent_decode(v)))
        .collect();

    let oid = params
        .get("oid")
        .ok_or_else(|| WindchillError::Other("No 'oid' parameter found in URL".into()))?
        .clone();

    let container_oid = params.get("ContainerOid").cloned();

    Ok(WindchillUrlParams { oid, container_oid })
}

/// Resolve a Windchill web URL to the canonical document OID used for upload operations.
pub fn resolve_document_oid(client: &WindchillClient, url: &str) -> Result<DocumentInfo> {
    let params = parse_document_url(url)?;

    let path = format!("/servlet/odata/DocMgmt/Documents('{}')", params.oid);
    let response = client.get(&path)?;
    let json: Value = response.json()?;

    let id = json["ID"]
        .as_str()
        .ok_or_else(|| WindchillError::Other("No ID field in document response".into()))?
        .to_string();

    let name = json["Name"].as_str().unwrap_or("Unknown").to_string();

    Ok(DocumentInfo { id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode_colons() {
        assert_eq!(
            percent_decode("OR%3Awt.inf.library.WTLibrary%3A1087756099"),
            "OR:wt.inf.library.WTLibrary:1087756099"
        );
    }

    #[test]
    fn test_percent_decode_already_decoded() {
        assert_eq!(
            percent_decode("VR:wt.doc.WTDocument:2034364227"),
            "VR:wt.doc.WTDocument:2034364227"
        );
    }

    #[test]
    fn test_percent_decode_mixed() {
        assert_eq!(percent_decode("hello%20world%21"), "hello world!");
    }

    #[test]
    fn test_parse_document_url_full() {
        let url = "https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?ContainerOid=OR%3Awt.inf.library.WTLibrary%3A1087756099&oid=VR%3Awt.doc.WTDocument%3A2034364227&u8=1";
        let params = parse_document_url(url).unwrap();

        assert_eq!(params.oid, "VR:wt.doc.WTDocument:2034364227");
        assert_eq!(
            params.container_oid.unwrap(),
            "OR:wt.inf.library.WTLibrary:1087756099"
        );
    }

    #[test]
    fn test_parse_document_url_no_container() {
        let url = "https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?oid=VR%3Awt.doc.WTDocument%3A2034364227&u8=1";
        let params = parse_document_url(url).unwrap();

        assert_eq!(params.oid, "VR:wt.doc.WTDocument:2034364227");
        assert!(params.container_oid.is_none());
    }

    #[test]
    fn test_parse_document_url_no_fragment() {
        let url = "https://windchill.example.com/Windchill/app/";
        let result = parse_document_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_document_url_no_query_params() {
        let url = "https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage";
        let result = parse_document_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_document_url_missing_oid() {
        let url = "https://windchill.example.com/Windchill/app/#ptc1/tcomp/infoPage?ContainerOid=OR%3Awt.inf.library.WTLibrary%3A1087756099&u8=1";
        let result = parse_document_url(url);
        assert!(result.is_err());
    }
}
