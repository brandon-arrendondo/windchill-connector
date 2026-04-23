use crate::error::{Result, WindchillError};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

pub struct WindchillClient {
    base_url: String,
    auth_token: String,
    client: Client,
}

impl WindchillClient {
    pub fn new(base_url: String, auth_token: String) -> Result<Self> {
        let client = Client::builder()
            .danger_accept_invalid_certs(false)
            .build()?;

        Ok(Self {
            base_url,
            auth_token,
            client,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {}", self.auth_token)).unwrap(),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert("Prefer", HeaderValue::from_static("odata.maxpagesize=100"));
        headers
    }

    pub fn get(&self, path: &str) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let headers = self.default_headers();

        let response = self.client.get(&url).headers(headers).send()?;

        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(WindchillError::AuthError(
                "Authentication failed. Please check your credentials.".to_string(),
            ));
        }

        Ok(response)
    }

    pub fn post(&self, path: &str, body: &Value, nonce: Option<&str>) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut headers = self.default_headers();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(nonce_value) = nonce {
            headers.insert("CSRF_NONCE", HeaderValue::from_str(nonce_value).unwrap());
        }

        let response = self.client.post(&url).headers(headers).json(body).send()?;

        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(WindchillError::AuthError(
                "Authentication failed. Please check your credentials.".to_string(),
            ));
        }

        Ok(response)
    }

    pub fn put_file(
        &self,
        path: &str,
        file_path: &std::path::Path,
        nonce: &str,
        timeout: std::time::Duration,
    ) -> Result<Response> {
        let url = format!("{}{}", self.base_url, path);
        let mut headers = self.default_headers();
        headers.insert("CSRF_NONCE", HeaderValue::from_str(nonce).unwrap());

        let form = reqwest::blocking::multipart::Form::new().file("src", file_path)?;

        let response = self
            .client
            .put(&url)
            .headers(headers)
            .multipart(form)
            .timeout(timeout)
            .send()?;

        if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
            return Err(WindchillError::AuthError(
                "Authentication failed during file upload.".to_string(),
            ));
        }

        Ok(response)
    }

    /// Get CSRF nonce token
    pub fn get_nonce(&self) -> Result<String> {
        let response = self.get("/servlet/odata/PTC/GetCSRFToken()")?;
        let json: Value = response.json()?;

        json["NonceValue"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| WindchillError::InvalidResponse("Failed to get nonce value".to_string()))
    }
}
