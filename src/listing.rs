use crate::client::WindchillClient;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subdirs: Option<Vec<TreeNode>>,
}

/// Fetch all pages of an OData collection, following @odata.nextLink pagination.
fn fetch_all_odata_values(client: &WindchillClient, initial_path: &str) -> Result<Vec<Value>> {
    let mut all_values = Vec::new();

    let response = client.get(initial_path)?;
    let mut json: Value = response.json()?;

    if let Some(arr) = json["value"].as_array() {
        all_values.extend(arr.clone());
    }

    while let Some(next_link) = json["@odata.nextLink"].as_str() {
        let path = if next_link.starts_with("http") {
            next_link
                .strip_prefix(client.base_url())
                .unwrap_or(next_link)
                .to_string()
        } else {
            next_link.to_string()
        };

        let response = client.get(&path)?;
        json = response.json()?;

        if let Some(arr) = json["value"].as_array() {
            all_values.extend(arr.clone());
        }
    }

    Ok(all_values)
}

/// Retrieve document metadata and primary content
pub fn retrieve_document_data(client: &WindchillClient, oid: &str) -> Result<(String, String)> {
    let path = format!("/servlet/odata/DocMgmt/Documents('{}')", oid);
    let doc_response = client.get(&path)?;
    let doc_data = doc_response.text()?;

    let pc_path = format!("{}/PrimaryContent", path);
    let pc_response = client.get(&pc_path)?;
    let primary_content_data = pc_response.text()?;

    Ok((doc_data, primary_content_data))
}

fn subfolder_dive(
    client: &WindchillClient,
    current_url: &str,
    values: Vec<Value>,
) -> Result<Vec<TreeNode>> {
    let mut subdirs = Vec::new();

    for subfolder in values {
        let name = subfolder["Name"].as_str().unwrap_or("Unknown").to_string();
        let id = subfolder["ID"].as_str().unwrap_or("").to_string();

        let updated_url = format!("{}('{}')/Folders", current_url, id);
        let arr = fetch_all_odata_values(client, &updated_url)?;

        let children = if arr.is_empty() {
            let contents_url = updated_url.replace("/Folders", "/Contents");
            let items = fetch_all_odata_values(client, &contents_url)?;

            if items.is_empty() {
                None
            } else {
                let mut content_nodes = Vec::new();
                for item in items {
                    let item_name = item["Name"].as_str().unwrap_or("Unknown").to_string();
                    let item_id = item["ID"].as_str().unwrap_or("").to_string();
                    content_nodes.push(TreeNode {
                        name: item_name,
                        id: item_id,
                        subdirs: None,
                    });
                }
                Some(content_nodes)
            }
        } else {
            let children = subfolder_dive(client, &updated_url, arr)?;
            if children.is_empty() {
                None
            } else {
                Some(children)
            }
        };

        subdirs.push(TreeNode {
            name,
            id,
            subdirs: children,
        });
    }

    Ok(subdirs)
}

/// Fetch the entire tree structure for a container
pub fn fetch_item_tree(client: &WindchillClient, oid: &str) -> Result<TreeNode> {
    let path = format!("/servlet/odata/DataAdmin/Containers('{}')", oid);
    let response = client.get(&path)?;
    let json: Value = response.json()?;

    let name = json["Name"].as_str().unwrap_or("Unknown").to_string();

    let folders_path = format!("{}/Folders", path);
    let values = fetch_all_odata_values(client, &folders_path)?;

    let subdirs = if values.is_empty() {
        None
    } else {
        let children = subfolder_dive(client, &folders_path, values)?;
        if children.is_empty() {
            None
        } else {
            Some(children)
        }
    };

    Ok(TreeNode {
        name,
        id: oid.to_string(),
        subdirs,
    })
}

pub fn print_tree(node: &TreeNode, prefix: &str, is_last: bool) {
    let connector = if is_last { "└── " } else { "├── " };
    println!("{}{}{}: {}", prefix, connector, node.name, node.id);

    if let Some(ref subdirs) = node.subdirs {
        let new_prefix = format!("{}{}   ", prefix, if is_last { " " } else { "│" });
        for (i, child) in subdirs.iter().enumerate() {
            print_tree(child, &new_prefix, i == subdirs.len() - 1);
        }
    }
}
