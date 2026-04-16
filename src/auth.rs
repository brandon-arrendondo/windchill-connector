use crate::error::{Result, WindchillError};
use base64::{engine::general_purpose, Engine};
use std::io::{self, Write};

/// Prompt for username and password, return base64-encoded Basic Auth token
pub fn prompt_for_credentials() -> Result<(String, String)> {
    print!("Username [{}]: ", whoami::username());
    io::stdout().flush()?;

    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    username = username.trim().to_string();

    if username.is_empty() {
        username = whoami::username();
    }

    let password = rpassword::prompt_password("Password: ")?;

    if password.is_empty() {
        return Err(WindchillError::AuthError(
            "Password cannot be empty".to_string(),
        ));
    }

    let auth_token = create_auth_token(&username, &password);

    Ok((username, auth_token))
}

/// Create a Basic Auth token from username and password
pub fn create_auth_token(username: &str, password: &str) -> String {
    let credentials = format!("{}:{}", username, password);
    general_purpose::STANDARD.encode(credentials.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_auth_token() {
        let token = create_auth_token("user", "pass");
        assert_eq!(token, "dXNlcjpwYXNz");
    }
}
