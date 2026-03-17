use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use reqwest::{blocking::ClientBuilder, Proxy, Url};
use serde::Serialize;

mod proxy_config;

const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const FALLBACK_PRICING_SOURCE_URL: &str = "bundled://codex-cost/default-pricing";
const FALLBACK_PRICING_FETCHED_AT: &str = "2026-03-17T00:00:00Z";

#[derive(Serialize)]
struct BundledPricingCache {
    fetched_at: String,
    source_url: String,
    prices: serde_json::Value,
}

fn main() {
    stage_bundled_pricing().expect("failed to stage bundled pricing");

    if cfg!(target_os = "windows") {
        stage_webview2_loader().expect("failed to stage WebView2Loader.dll");
    }

    tauri_build::build();
}

fn stage_bundled_pricing() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let fallback_path = manifest_dir.join("assets").join("default-pricing.json");
    let bundled_path = out_dir.join("bundled-pricing.json");

    let fetch_attempt = fetch_remote_pricing();
    let cache = match fetch_attempt {
        Ok(prices) => BundledPricingCache {
            fetched_at: Utc::now().to_rfc3339(),
            source_url: LITELLM_PRICING_URL.to_string(),
            prices,
        },
        Err(error) => {
            println!(
                "cargo:warning=failed to fetch LiteLLM pricing during build, using fallback: {error}"
            );
            let content = fs::read_to_string(&fallback_path)?;
            let prices = serde_json::from_str(&content)?;
            BundledPricingCache {
                fetched_at: FALLBACK_PRICING_FETCHED_AT.to_string(),
                source_url: FALLBACK_PRICING_SOURCE_URL.to_string(),
                prices,
            }
        }
    };

    let content = serde_json::to_string_pretty(&cache)?;
    fs::write(&bundled_path, content)?;

    println!("cargo:rerun-if-changed={}", fallback_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}

fn fetch_remote_pricing() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let client = pricing_client_builder()?
        .timeout(Duration::from_secs(15))
        .build()?;
    let response = client.get(LITELLM_PRICING_URL).send()?.error_for_status()?;
    Ok(response.json()?)
}

fn pricing_client_builder() -> Result<ClientBuilder, Box<dyn std::error::Error>> {
    let mut builder = reqwest::blocking::Client::builder();
    if proxy_config::explicit_proxy_env_is_set() {
        return Ok(builder);
    }

    let Some(config) = proxy_config::load_system_proxy_config() else {
        return Ok(builder);
    };

    let target_url = Url::parse(LITELLM_PRICING_URL)?;
    let Some(host) = target_url.host_str() else {
        return Ok(builder);
    };

    if proxy_config::should_bypass_proxy(host, &config.exceptions) {
        return Ok(builder);
    }

    if let Some(proxy_url) = proxy_url_for_target(&target_url, &config) {
        builder = builder.proxy(Proxy::all(&proxy_url)?);
    }

    Ok(builder)
}

fn proxy_url_for_target(
    target_url: &Url,
    config: &proxy_config::SystemProxyConfig,
) -> Option<String> {
    let endpoint = match target_url.scheme() {
        "https" => config
            .https
            .as_ref()
            .or(config.http.as_ref())
            .or(config.socks.as_ref()),
        "http" => config
            .http
            .as_ref()
            .or(config.https.as_ref())
            .or(config.socks.as_ref()),
        _ => config
            .socks
            .as_ref()
            .or(config.https.as_ref())
            .or(config.http.as_ref()),
    }?;

    let scheme = if target_url.scheme() == "https" && config.https.as_ref() == Some(endpoint) {
        "http"
    } else if target_url.scheme() == "http" && config.http.as_ref() == Some(endpoint) {
        "http"
    } else if config.socks.as_ref() == Some(endpoint) {
        "socks5h"
    } else {
        "http"
    };

    Some(format!("{scheme}://{}:{}", endpoint.host, endpoint.port))
}

fn stage_webview2_loader() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .ok_or("failed to resolve target profile dir from OUT_DIR")?;

    let target = env::var("TARGET")?;
    let arch_dir = if target.contains("x86_64") {
        "x64"
    } else if target.contains("aarch64") {
        "arm64"
    } else if target.contains("i686") {
        "x86"
    } else {
        return Err(format!("unsupported windows target: {target}").into());
    };

    let source = find_webview2_loader(profile_dir, arch_dir)
        .ok_or("unable to locate WebView2Loader.dll in cargo build output")?;
    let staged = profile_dir.join("WebView2Loader.dll");
    fs::copy(&source, &staged)?;

    println!("cargo:rerun-if-changed={}", source.display());
    println!(
        "cargo:warning=staged WebView2Loader.dll from {}",
        source.display()
    );

    Ok(())
}

fn find_webview2_loader(profile_dir: &Path, arch_dir: &str) -> Option<PathBuf> {
    let build_dir = profile_dir.join("build");
    let entries = fs::read_dir(build_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy();
        if !name.starts_with("webview2-com-sys-") {
            continue;
        }

        let candidate = path.join("out").join(arch_dir).join("WebView2Loader.dll");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}
