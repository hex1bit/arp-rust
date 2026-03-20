use crate::config::{AuthConfig, AuthRule, PortRange};
use crate::error::{Error, Result};
use crate::protocol::{LoginMsg, NewProxyMsg, NewWorkConnMsg, PingMsg};

pub trait Authenticator: Send + Sync {
    fn verify_login(&self, msg: &LoginMsg) -> Result<()>;
    fn verify_new_work_conn(&self, msg: &NewWorkConnMsg) -> Result<()>;
    fn verify_ping(&self, msg: &PingMsg) -> Result<()>;
    fn authorize_proxy(&self, privilege_key: &str, pool_count: u32, msg: &NewProxyMsg) -> Result<()>;
}

pub struct TokenAuth {
    token: String,
    additional_tokens: Vec<String>,
    rules: Vec<AuthRule>,
}

impl TokenAuth {
    pub fn new(config: &AuthConfig) -> Self {
        Self {
            token: config.token.clone(),
            additional_tokens: config.additional_tokens.clone(),
            rules: config.rules.clone(),
        }
    }

    fn is_known_token(&self, token: &str) -> bool {
        token == self.token
            || self.additional_tokens.iter().any(|item| item == token)
            || self.rules.iter().any(|rule| rule.token == token)
    }

    fn matching_rule(&self, token: &str) -> Option<&AuthRule> {
        self.rules.iter().find(|rule| rule.token == token)
    }

    fn validate_rule_login(&self, rule: &AuthRule, msg: &LoginMsg) -> Result<()> {
        if rule.max_pool_count > 0 && msg.pool_count > rule.max_pool_count {
            return Err(Error::Auth(format!(
                "pool_count {} exceeds token rule max_pool_count {}",
                msg.pool_count, rule.max_pool_count
            )));
        }
        Ok(())
    }

    fn validate_rule_proxy(&self, rule: &AuthRule, pool_count: u32, msg: &NewProxyMsg) -> Result<()> {
        if rule.max_pool_count > 0 && pool_count > rule.max_pool_count {
            return Err(Error::Auth(format!(
                "pool_count {} exceeds token rule max_pool_count {}",
                pool_count, rule.max_pool_count
            )));
        }

        if !rule.allow_proxy_types.is_empty()
            && !rule
                .allow_proxy_types
                .iter()
                .any(|item| item.eq_ignore_ascii_case(&msg.proxy_type))
        {
            return Err(Error::Auth(format!(
                "proxy type {} is not allowed for this token",
                msg.proxy_type
            )));
        }

        if !rule.allow_ports.is_empty() {
            if msg.remote_port == 0 {
                return Err(Error::Auth(
                    "this token rule requires a fixed remote_port within allowed ranges"
                        .to_string(),
                ));
            }
            if !port_in_ranges(msg.remote_port, &rule.allow_ports) {
                return Err(Error::Auth(format!(
                    "remote_port {} is not allowed for this token",
                    msg.remote_port
                )));
            }
        }

        if !rule.allow_domain_suffixes.is_empty() {
            for domain in &msg.custom_domains {
                let normalized = normalize_host(domain);
                if !rule
                    .allow_domain_suffixes
                    .iter()
                    .any(|suffix| normalized.ends_with(&normalize_host(suffix)))
                {
                    return Err(Error::Auth(format!(
                        "domain {} is not allowed for this token",
                        domain
                    )));
                }
            }
        }

        if !msg.subdomain.trim().is_empty() && !rule.allow_subdomain_prefixes.is_empty() {
            let subdomain = msg.subdomain.trim().to_lowercase();
            if !rule
                .allow_subdomain_prefixes
                .iter()
                .any(|prefix| subdomain.starts_with(&prefix.trim().to_lowercase()))
            {
                return Err(Error::Auth(format!(
                    "subdomain {} is not allowed for this token",
                    msg.subdomain
                )));
            }
        }

        Ok(())
    }
}

impl Authenticator for TokenAuth {
    fn verify_login(&self, msg: &LoginMsg) -> Result<()> {
        if !self.is_known_token(&msg.privilege_key) {
            return Err(Error::Auth("Invalid token".to_string()));
        }
        if let Some(rule) = self.matching_rule(&msg.privilege_key) {
            self.validate_rule_login(rule, msg)?;
        }
        Ok(())
    }

    fn verify_new_work_conn(&self, msg: &NewWorkConnMsg) -> Result<()> {
        if !self.is_known_token(&msg.privilege_key) {
            return Err(Error::Auth("Invalid token".to_string()));
        }
        Ok(())
    }

    fn verify_ping(&self, _msg: &PingMsg) -> Result<()> {
        Ok(())
    }

    fn authorize_proxy(&self, privilege_key: &str, pool_count: u32, msg: &NewProxyMsg) -> Result<()> {
        if !self.is_known_token(privilege_key) {
            return Err(Error::Auth("Invalid token".to_string()));
        }
        if let Some(rule) = self.matching_rule(privilege_key) {
            self.validate_rule_proxy(rule, pool_count, msg)?;
        }
        Ok(())
    }
}

fn port_in_ranges(port: u16, ranges: &[PortRange]) -> bool {
    ranges.iter().any(|range| {
        if range.single > 0 {
            port == range.single
        } else {
            port >= range.start && port <= range.end
        }
    })
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_lowercase()
}

pub fn create_authenticator(config: &AuthConfig) -> Result<Box<dyn Authenticator>> {
    match config.method.as_str() {
        "token" => Ok(Box::new(TokenAuth::new(config))),
        other => Err(Error::Config(format!(
            "unsupported auth.method: {}",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OidcConfig;

    fn token_config() -> AuthConfig {
        AuthConfig {
            method: "token".to_string(),
            token: "root-token".to_string(),
            additional_tokens: vec!["extra-token".to_string()],
            rules: vec![AuthRule {
                token: "scoped-token".to_string(),
                allow_proxy_types: vec!["tcp".to_string(), "http".to_string()],
                allow_ports: vec![PortRange {
                    start: 6000,
                    end: 6010,
                    single: 0,
                }],
                allow_domain_suffixes: vec!["example.com".to_string()],
                allow_subdomain_prefixes: vec!["team-".to_string()],
                max_pool_count: 2,
            }],
            oidc: OidcConfig::default(),
        }
    }

    fn login(token: &str, pool_count: u32) -> LoginMsg {
        LoginMsg {
            version: "0.1.0".to_string(),
            hostname: "test".to_string(),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            user: "test".to_string(),
            client_id: "client-a".to_string(),
            timestamp: 123456,
            privilege_key: token.to_string(),
            run_id: "".to_string(),
            pool_count,
        }
    }

    fn proxy_msg(proxy_type: &str, remote_port: u16) -> NewProxyMsg {
        NewProxyMsg {
            proxy_name: "p1".to_string(),
            proxy_type: proxy_type.to_string(),
            use_encryption: false,
            use_compression: false,
            local_ip: "127.0.0.1".to_string(),
            local_port: 8080,
            remote_port,
            custom_domains: vec![],
            subdomain: String::new(),
            locations: vec![],
            host_header_rewrite: String::new(),
            sk: String::new(),
            multiplexer: String::new(),
            fallback_to_relay: true,
            extra: serde_json::json!({}),
        }
    }

    #[test]
    fn test_token_auth_valid() {
        let auth = TokenAuth::new(&token_config());
        assert!(auth.verify_login(&login("root-token", 1)).is_ok());
        assert!(auth.verify_login(&login("extra-token", 1)).is_ok());
    }

    #[test]
    fn test_token_auth_invalid() {
        let auth = TokenAuth::new(&token_config());
        assert!(auth.verify_login(&login("wrong-token", 1)).is_err());
    }

    #[test]
    fn test_rule_enforces_pool_count() {
        let auth = TokenAuth::new(&token_config());
        assert!(auth.verify_login(&login("scoped-token", 3)).is_err());
        assert!(auth.verify_login(&login("scoped-token", 2)).is_ok());
    }

    #[test]
    fn test_rule_enforces_proxy_type_and_port() {
        let auth = TokenAuth::new(&token_config());
        assert!(auth
            .authorize_proxy("scoped-token", 1, &proxy_msg("udp", 6001))
            .is_err());
        assert!(auth
            .authorize_proxy("scoped-token", 1, &proxy_msg("tcp", 7001))
            .is_err());
        assert!(auth
            .authorize_proxy("scoped-token", 1, &proxy_msg("tcp", 6001))
            .is_ok());
    }

    #[test]
    fn test_rule_enforces_domain_and_subdomain() {
        let auth = TokenAuth::new(&token_config());
        let mut msg = proxy_msg("http", 6001);
        msg.custom_domains = vec!["bad.other.net".to_string()];
        assert!(auth.authorize_proxy("scoped-token", 1, &msg).is_err());

        msg.custom_domains = vec!["app.example.com".to_string()];
        assert!(auth.authorize_proxy("scoped-token", 1, &msg).is_ok());

        msg.custom_domains.clear();
        msg.subdomain = "team-api".to_string();
        assert!(auth.authorize_proxy("scoped-token", 1, &msg).is_ok());

        msg.subdomain = "other-api".to_string();
        assert!(auth.authorize_proxy("scoped-token", 1, &msg).is_err());
    }
}
