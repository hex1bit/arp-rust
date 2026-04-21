use std::net::TcpStream as StdTcpStream;
use std::time::{Duration, Instant};

use arp_common::config::ClientConfig;

/// Run `arpc status` — show connection and proxy status.
pub async fn run_status(config: &ClientConfig) {
    let admin_addr = effective_admin_addr(config);
    let url = format!("http://{}/api/v1/status", admin_addr);

    match http_get(&url, config).await {
        Ok(body) => {
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&body) {
                println!("ARP Client Status");
                println!("─────────────────────────────────────────");
                println!(
                    "  Status:      {}",
                    status["status"].as_str().unwrap_or("unknown")
                );
                println!(
                    "  Server:      {}:{}",
                    status["server_addr"].as_str().unwrap_or("?"),
                    status["server_port"].as_u64().unwrap_or(0)
                );
                println!(
                    "  Run ID:      {}",
                    status["run_id"].as_str().unwrap_or("-")
                );
                if let Some(proxies) = status["proxies"].as_array() {
                    println!("  Proxies:     {}", proxies.len());
                    for p in proxies {
                        println!("    - {}", p.as_str().unwrap_or("?"));
                    }
                }
            } else {
                println!("{}", body);
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to admin API at {}: {}", admin_addr, e);
            eprintln!("Make sure arpc is running with admin_port configured.");
        }
    }
}

/// Run `arpc check [name]` — diagnose proxy connectivity.
pub async fn run_check(config: &ClientConfig, name: Option<String>) {
    let admin_addr = effective_admin_addr(config);

    // Step 1: Check admin API reachable
    print!("[.] Admin API ({})...", admin_addr);
    let status_url = format!("http://{}/api/v1/status", admin_addr);
    let status_body = match http_get(&status_url, config).await {
        Ok(body) => {
            println!("\r[✓] Admin API ({})", admin_addr);
            body
        }
        Err(e) => {
            println!("\r[✗] Admin API ({}) — {}", admin_addr, e);
            eprintln!("    arpc may not be running or admin_port is not configured.");
            return;
        }
    };

    // Step 2: Check control connection
    if let Ok(status) = serde_json::from_str::<serde_json::Value>(&status_body) {
        let conn_status = status["status"].as_str().unwrap_or("unknown");
        if conn_status == "connected" {
            println!(
                "[✓] Control connection: {} (run_id={})",
                conn_status,
                status["run_id"].as_str().unwrap_or("-")
            );
        } else {
            println!("[✗] Control connection: {}", conn_status);
        }

        // Step 3: If a specific proxy name given, check it
        if let Some(proxy_name) = name {
            let proxy_url = format!(
                "http://{}/api/v1/proxies/{}",
                admin_addr, proxy_name
            );
            match http_get(&proxy_url, config).await {
                Ok(body) => {
                    if let Ok(detail) = serde_json::from_str::<serde_json::Value>(&body) {
                        if detail.get("available").is_some() {
                            let available = detail["available"].as_bool().unwrap_or(false);
                            if available {
                                println!("[✓] Proxy '{}': registered and available", proxy_name);
                            } else {
                                println!("[✗] Proxy '{}': registered but NOT available", proxy_name);
                            }
                        } else {
                            println!("[✗] Proxy '{}': not found", proxy_name);
                        }
                    }
                }
                Err(e) => {
                    println!("[✗] Proxy '{}': {}", proxy_name, e);
                }
            }

            // Step 4: Check local service reachability for this proxy
            check_local_service(config, &proxy_name);
        } else {
            // Check all proxies
            if let Some(proxies) = status["proxies"].as_array() {
                for p in proxies {
                    if let Some(pname) = p.as_str() {
                        check_local_service(config, pname);
                    }
                }
            }
        }
    }
}

fn check_local_service(config: &ClientConfig, proxy_name: &str) {
    if let Some(proxy) = config.proxies.iter().find(|p| p.name == proxy_name) {
        let addr = format!("{}:{}", proxy.local_ip, proxy.local_port);
        let start = Instant::now();
        match StdTcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| ([127, 0, 0, 1], proxy.local_port).into()),
            Duration::from_secs(3),
        ) {
            Ok(_) => {
                let ms = start.elapsed().as_millis();
                println!(
                    "[✓] Local service '{}' ({}): reachable ({}ms)",
                    proxy_name, addr, ms
                );
            }
            Err(e) => {
                println!(
                    "[✗] Local service '{}' ({}): {} ",
                    proxy_name, addr, e
                );
            }
        }
    }
}

fn effective_admin_addr(config: &ClientConfig) -> String {
    let addr = if config.admin_addr.is_empty() {
        "127.0.0.1"
    } else {
        &config.admin_addr
    };
    format!("{}:{}", addr, config.admin_port)
}

async fn http_get(url: &str, config: &ClientConfig) -> std::result::Result<String, String> {
    hyper_util_get(url, config).await
}

/// Minimal HTTP GET using tokio TcpStream (no extra HTTP client dependency).
async fn hyper_util_get(url: &str, config: &ClientConfig) -> std::result::Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let url = url
        .strip_prefix("http://")
        .ok_or_else(|| "invalid URL".to_string())?;
    let (host_port, path) = url.split_once('/').unwrap_or((url, ""));
    let path = format!("/{}", path);

    let mut stream = tokio::net::TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("connect failed: {}", e))?;

    let mut request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        path, host_port
    );
    if !config.admin_user.is_empty() && !config.admin_pwd.is_empty() {
        let credentials = format!("{}:{}", config.admin_user, config.admin_pwd);
        let encoded = base64_encode(credentials.as_bytes());
        request.push_str(&format!("Authorization: Basic {}\r\n", encoded));
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write failed: {}", e))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| format!("read failed: {}", e))?;

    let response_str = String::from_utf8_lossy(&response);
    // Extract body after \r\n\r\n
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let status_line = response_str.lines().next().unwrap_or("");
        if status_line.contains("401") {
            return Err("unauthorized (check admin_user/admin_pwd)".to_string());
        }
        Ok(response_str[pos + 4..].to_string())
    } else {
        Err("invalid HTTP response".to_string())
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        output.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            output.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
        if i + 2 < input.len() {
            output.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            output.push('=');
        }
        i += 3;
    }
    output
}
