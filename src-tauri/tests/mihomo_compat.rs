use serde_json::{Value, json};
use tauri_plugin_mihomo::models::{BaseConfig, ClashMode, Proxies, ProxyType};

fn config_response() -> Value {
    json!({
        "port": 0, "socks-port": 0, "redir-port": 0, "tproxy-port": 0, "mixed-port": 7897,
        "tun": { "enable": false, "device": "", "stack": "Mixed", "dns-hijack": [],
            "auto-route": false, "auto-detect-interface": false, "file-descriptor": 0 },
        "tuic-server": { "enable": false, "listen": "", "certificate": "", "private-key": "", "ech-key": "" },
        "ss-config": "", "vmess-config": "", "allow-lan": false, "bind-address": "*",
        "inbound-tfo": false, "inbound-mptcp": false, "mode": "rule", "unified-delay": false,
        "log-level": "info", "ipv6": false, "interface-name": "", "routing-mark": 0,
        "geox-url": { "geo-ip": "", "mmdb": "", "asn": "", "geo-site": "" },
        "geo-auto-update": false, "geo-update-interval": 24, "geodata-mode": false,
        "geodata-loader": "memconservative", "geosite-matcher": "succinct", "tcp-concurrent": false,
        "find-process-mode": "strict", "sniffing": false, "global-ua": "mihomo",
        "etag-support": true, "keep-alive-interval": 15, "keep-alive-idle": 15, "disable-keep-alive": false
    })
}

fn proxy_response(kind: &str) -> Value {
    json!({ "name": "test", "type": kind, "alive": true, "history": [], "extra": {},
        "udp": true, "uot": false, "xudp": false, "tfo": false, "mptcp": false,
        "smux": false, "interface": "", "dialer-proxy": "", "routing-mark": 0 })
}

#[test]
fn config_without_removed_fingerprint_is_accepted() -> anyhow::Result<()> {
    let config: BaseConfig = serde_json::from_value(config_response())?;
    assert_eq!(config.mode, ClashMode::Rule);
    assert_eq!(config.mixed_port, 7897);
    assert!(config.global_client_fingerprint.is_empty());
    let serialized = serde_json::to_value(config)?;
    assert_eq!(serialized["mixedPort"], 7897);
    Ok(())
}

#[test]
fn old_fingerprint_is_preserved_and_required_fields_stay_required() -> anyhow::Result<()> {
    let mut response = config_response();
    response["global-client-fingerprint"] = json!("chrome");
    let config: BaseConfig = serde_json::from_value(response.clone())?;
    assert_eq!(config.global_client_fingerprint, "chrome");
    response
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("配置不是对象"))?
        .remove("mode");
    assert!(serde_json::from_value::<BaseConfig>(response).is_err());
    Ok(())
}

#[test]
fn new_proxy_types_do_not_break_the_collection() -> anyhow::Result<()> {
    let response = json!({ "proxies": {
        "pass": proxy_response("PassRule"),
        "future": proxy_response("FutureProxy"),
        "direct": proxy_response("Direct")
    }});
    let proxies: Proxies = serde_json::from_value(response)?;
    assert_eq!(proxies.proxies.len(), 3);
    assert_eq!(proxies.proxies["pass"].proxy_type, ProxyType::PassRule);
    assert_eq!(proxies.proxies["future"].proxy_type, ProxyType::Unknown);
    assert_eq!(proxies.proxies["direct"].proxy_type, ProxyType::Direct);
    Ok(())
}

// 显式执行时仅发送 GET 请求，不启动、停止或更新用户内核。
#[cfg(windows)]
#[tokio::test]
#[ignore = "需要本机已运行的内核，CI 不自动执行"]
async fn installed_core_responses_are_compatible() -> anyhow::Result<()> {
    use tauri_plugin_mihomo::{IpcConnectionPool, IpcPoolConfigBuilder, Mihomo, models::Protocol};
    IpcConnectionPool::init(IpcPoolConfigBuilder::new().build())?;
    let client = Mihomo {
        protocol: Protocol::LocalSocket,
        external_host: None,
        external_port: None,
        secret: None,
        socket_path: Some(r"\\.\pipe\verge-mihomo".into()),
        connection_manager: Default::default(),
    };
    let version = client.get_version().await?;
    let config = client.get_base_config().await?;
    let proxies = client.get_proxies().await?;
    client.get_proxy_providers().await?;
    println!(
        "内核版本={}，模式={}，代理项数={}",
        version.version,
        config.mode,
        proxies.proxies.len()
    );
    assert!(!proxies.proxies.is_empty());
    Ok(())
}
