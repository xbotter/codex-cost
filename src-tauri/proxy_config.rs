use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemProxyConfig {
    pub http: Option<ProxyEndpoint>,
    pub https: Option<ProxyEndpoint>,
    pub socks: Option<ProxyEndpoint>,
    pub exceptions: Vec<String>,
}

pub fn explicit_proxy_env_is_set() -> bool {
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

#[cfg(target_os = "macos")]
pub fn load_system_proxy_config() -> Option<SystemProxyConfig> {
    let output = Command::new("scutil").arg("--proxy").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    Some(parse_scutil_proxy_output(&stdout))
}

#[cfg(not(target_os = "macos"))]
pub fn load_system_proxy_config() -> Option<SystemProxyConfig> {
    None
}

pub fn parse_scutil_proxy_output(output: &str) -> SystemProxyConfig {
    let mut config = SystemProxyConfig::default();
    let mut lines = output.lines().peekable();

    while let Some(raw_line) = lines.next() {
        let line = raw_line.trim();
        if let Some((key, value)) = parse_key_value(line) {
            match key {
                "HTTPEnable" if value == "1" => {
                    config.http = parse_endpoint(output, "HTTPProxy", "HTTPPort");
                }
                "HTTPSEnable" if value == "1" => {
                    config.https = parse_endpoint(output, "HTTPSProxy", "HTTPSPort");
                }
                "SOCKSEnable" if value == "1" => {
                    config.socks = parse_endpoint(output, "SOCKSProxy", "SOCKSPort");
                }
                "ExceptionsList" if value == "<array> {" => {
                    while let Some(next_line) = lines.peek() {
                        let item = next_line.trim();
                        if item == "}" {
                            lines.next();
                            break;
                        }

                        if let Some((_idx, value)) = parse_key_value(item) {
                            config.exceptions.push(value.to_string());
                        }
                        lines.next();
                    }
                }
                _ => {}
            }
        }
    }

    config
}

pub fn should_bypass_proxy(host: &str, exceptions: &[String]) -> bool {
    exceptions
        .iter()
        .any(|pattern| matches_proxy_exception(host, pattern))
}

fn parse_endpoint(output: &str, host_key: &str, port_key: &str) -> Option<ProxyEndpoint> {
    let host = find_scalar_value(output, host_key)?;
    let port = find_scalar_value(output, port_key)?.parse().ok()?;
    Some(ProxyEndpoint { host, port })
}

fn find_scalar_value(output: &str, key: &str) -> Option<String> {
    output.lines().find_map(|raw_line| {
        let line = raw_line.trim();
        let (current_key, value) = parse_key_value(line)?;
        (current_key == key).then(|| value.to_string())
    })
}

fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

fn matches_proxy_exception(host: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }

    if let Some(suffix) = pattern.strip_prefix('.') {
        return host.ends_with(suffix);
    }

    host.eq_ignore_ascii_case(pattern)
}

#[cfg(test)]
mod tests {
    use super::{parse_scutil_proxy_output, should_bypass_proxy, ProxyEndpoint, SystemProxyConfig};

    #[test]
    fn parse_scutil_proxy_output_extracts_https_and_exceptions() {
        let output = r#"<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : localhost
  }
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : corp-http.local
  HTTPSEnable : 1
  HTTPSPort : 8443
  HTTPSProxy : corp-https.local
  SOCKSEnable : 1
  SOCKSPort : 1080
  SOCKSProxy : corp-socks.local
}"#;

        assert_eq!(
            parse_scutil_proxy_output(output),
            SystemProxyConfig {
                http: Some(ProxyEndpoint {
                    host: "corp-http.local".to_string(),
                    port: 8080,
                }),
                https: Some(ProxyEndpoint {
                    host: "corp-https.local".to_string(),
                    port: 8443,
                }),
                socks: Some(ProxyEndpoint {
                    host: "corp-socks.local".to_string(),
                    port: 1080,
                }),
                exceptions: vec!["*.local".to_string(), "localhost".to_string()],
            }
        );
    }

    #[test]
    fn should_bypass_proxy_matches_exact_and_wildcard_hosts() {
        let exceptions = vec!["*.local".to_string(), "localhost".to_string()];

        assert!(should_bypass_proxy("api.local", &exceptions));
        assert!(should_bypass_proxy("localhost", &exceptions));
        assert!(!should_bypass_proxy(
            "raw.githubusercontent.com",
            &exceptions
        ));
    }
}
