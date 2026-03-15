use crate::config::AuthConfig;
use crate::error::{Error, Result};
use crate::protocol::{LoginMsg, NewWorkConnMsg, PingMsg};

pub trait Authenticator: Send + Sync {
    fn verify_login(&self, msg: &LoginMsg) -> Result<()>;
    fn verify_new_work_conn(&self, msg: &NewWorkConnMsg) -> Result<()>;
    fn verify_ping(&self, msg: &PingMsg) -> Result<()>;
}

pub struct TokenAuth {
    token: String,
}

impl TokenAuth {
    pub fn new(token: String) -> Self {
        Self { token }
    }
}

impl Authenticator for TokenAuth {
    fn verify_login(&self, msg: &LoginMsg) -> Result<()> {
        if msg.privilege_key != self.token {
            return Err(Error::Auth("Invalid token".to_string()));
        }
        Ok(())
    }

    fn verify_new_work_conn(&self, msg: &NewWorkConnMsg) -> Result<()> {
        if !self.token.is_empty() && msg.privilege_key != self.token {
            return Err(Error::Auth("Invalid token".to_string()));
        }
        Ok(())
    }

    fn verify_ping(&self, _msg: &PingMsg) -> Result<()> {
        Ok(())
    }
}

pub fn create_authenticator(config: &AuthConfig) -> Box<dyn Authenticator> {
    match config.method.as_str() {
        "token" => Box::new(TokenAuth::new(config.token.clone())),
        _ => Box::new(TokenAuth::new(String::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_auth_valid() {
        let auth = TokenAuth::new("test-token".to_string());
        let msg = LoginMsg {
            version: "0.1.0".to_string(),
            hostname: "test".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            user: "test".to_string(),
            timestamp: 123456,
            privilege_key: "test-token".to_string(),
            run_id: "".to_string(),
            pool_count: 0,
        };

        assert!(auth.verify_login(&msg).is_ok());
    }

    #[test]
    fn test_token_auth_invalid() {
        let auth = TokenAuth::new("test-token".to_string());
        let msg = LoginMsg {
            version: "0.1.0".to_string(),
            hostname: "test".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            user: "test".to_string(),
            timestamp: 123456,
            privilege_key: "wrong-token".to_string(),
            run_id: "".to_string(),
            pool_count: 0,
        };

        assert!(auth.verify_login(&msg).is_err());
    }
}
