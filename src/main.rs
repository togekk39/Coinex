use anyhow::{anyhow, Context, Result};
use clap::{ArgAction, ArgGroup, Parser};
use chrono::{DateTime, Utc};
use dirs::cache_dir;
use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, time::Duration};
use tokio::time::sleep;

#[derive(Parser, Debug)]
#[command(
    name="CoinEx",
    version,
    about="即時加密貨幣↔法幣、幣↔幣換算 (CoinGecko API)",
    // 來源必須二選一：--crypto 或 --fiat
    group = ArgGroup::new("src").required(true).args(&["crypto","fiat"]),
    // 目標必須二選一：--to-crypto 或 --to-fiat
    group = ArgGroup::new("dst").required(true).args(&["to-crypto","to-fiat"])
)]
struct Cli {
    /// 來源為「加密貨幣」時指定（id/symbol/name：tia/sol/bitcoin/celestia/solana）
    #[arg(long, group="src")]
    crypto: Option<String>,

    /// 來源為「法幣/計價幣」時指定（vs 代碼：usd/eur/twd/btc/eth…）
    #[arg(long, group="src")]
    fiat: Option<String>,

    /// 要換算的數量
    amount: f64,

    /// 目標為「加密貨幣」時指定
    #[arg(long = "to-crypto", group="dst")]
    to_crypto: Option<String>,

    /// 目標為「法幣/計價幣」時指定
    #[arg(long = "to-fiat", group="dst")]
    to_fiat: Option<String>,

    /// 強制重新下載幣清單快取
    #[arg(long, action=ArgAction::SetTrue)]
    refresh: bool,

    /// 幣清單快取有效時間（例如：12h、1d；預設：1d）
    #[arg(long, default_value = "1d")]
    cache_ttl: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct CoinInfo {
    id: String,
    symbol: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedCoins {
    fetched_at: DateTime<Utc>,
    coins: Vec<CoinInfo>,
}

const COINGECKO_BASE: &str = "https://api.coingecko.com/api/v3";

// 熱門別名 → 正確 CoinGecko id（避免重名誤判）
fn alias_coin_id(sym_or_id: &str) -> Option<&'static str> {
    match sym_or_id.to_ascii_lowercase().as_str() {
        "tia" => Some("celestia"),
        "sol" => Some("solana"),
        "eth" => Some("ethereum"),
        "btc" => Some("bitcoin"),
        "ltc" => Some("litecoin"),
        "bch" => Some("bitcoin-cash"),
        "ada" => Some("cardano"),
        "xrp" => Some("ripple"),
        "dot" => Some("polkadot"),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let ttl = humantime::parse_duration(&cli.cache_ttl)
        .with_context(|| "無法解析 --cache-ttl，範例：12h、1d")?;

    let client = build_client()?;

    // 準備資料：幣清單 + 可支援的 vs（法幣/部分加密計價）
    let coins = get_or_fetch_coins(&client, cli.refresh, ttl).await?;
    let vs_set = fetch_supported_vs(&client).await.unwrap_or_default();

    // 解析來源與目標
    let src = if let Some(sym) = &cli.crypto {
        AssetKind::CoinId(resolve_coin_id_strict(&client, &coins, sym).await?)
    } else {
        let vs = cli.fiat.as_ref().unwrap().to_lowercase();
        ensure_vs(&vs_set, &vs)?;
        AssetKind::VsCurrency(vs)
    };

    let dst = if let Some(sym) = &cli.to_crypto {
        AssetKind::CoinId(resolve_coin_id_strict(&client, &coins, sym).await?)
    } else {
        let vs = cli.to_fiat.as_ref().unwrap().to_lowercase();
        ensure_vs(&vs_set, &vs)?;
        AssetKind::VsCurrency(vs)
    };

    // 計算
    let value = convert_pair(&client, &src, &dst, cli.amount).await?;
    println!("{} {} -> {} {}", cli.amount, src.display(), value, dst.display());
    Ok(())
}

fn build_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("CoinEx/0.4 (+https://github.com/)"),
    );
    if let Ok(key) = std::env::var("COINGECKO_API_KEY") {
        if !key.trim().is_empty() {
            headers.insert(
                "x-cg-demo-api-key",
                HeaderValue::from_str(key.trim()).unwrap_or_else(|_| HeaderValue::from_static("")),
            );
        }
    }

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .http2_adaptive_window(true)
        .pool_idle_timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    Ok(client)
}

fn cache_path() -> Result<PathBuf> {
    let mut p = cache_dir().ok_or_else(|| anyhow!("無法取得系統快取目錄"))?;
    p.push("CoinEx");
    fs::create_dir_all(&p).ok();
    p.push("coins.json");
    Ok(p)
}

async fn get_or_fetch_coins(
    client: &reqwest::Client,
    refresh: bool,
    ttl: Duration,
) -> Result<Vec<CoinInfo>> {
    let path = cache_path()?;

    if !refresh {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(cached) = serde_json::from_slice::<CachedCoins>(&bytes) {
                if Utc::now()
                    .signed_duration_since(cached.fetched_at)
                    .to_std()
                    .unwrap_or(Duration::from_secs(u64::MAX))
                    <= ttl
                {
                    return Ok(cached.coins);
                }
            }
        }
    }

    let coins = fetch_coins_list(client).await?;
    let cached = CachedCoins {
        fetched_at: Utc::now(),
        coins: coins.clone(),
    };
    let _ = fs::write(&path, serde_json::to_vec_pretty(&cached)?);
    Ok(coins)
}

async fn fetch_coins_list(client: &reqwest::Client) -> Result<Vec<CoinInfo>> {
    let url = format!("{}/coins/list?include_platform=false", COINGECKO_BASE);
    let resp = request_with_retry(client, client.get(&url)).await?;
    let coins: Vec<CoinInfo> = resp.json().await?;
    if coins.is_empty() {
        return Err(anyhow!("CoinGecko coins/list 回傳為空"));
    }
    Ok(coins)
}

async fn fetch_supported_vs(client: &reqwest::Client) -> Result<std::collections::HashSet<String>> {
    let url = format!("{}/simple/supported_vs_currencies", COINGECKO_BASE);
    let resp = request_with_retry(client, client.get(&url)).await?;
    let arr: Vec<String> = resp.json().await?;
    Ok(arr.into_iter().map(|s| s.to_lowercase()).collect())
}

fn ensure_vs(vs_set: &std::collections::HashSet<String>, vs: &str) -> Result<()> {
    if !vs_set.contains(vs) {
        return Err(anyhow!(
            "不支援的法幣/計價代碼：'{}'。可用清單含常見的 usd/eur/twd/btc/eth 等",
            vs
        ));
    }
    Ok(())
}

enum AssetKind {
    CoinId(String),
    VsCurrency(String), // e.g. usd, twd, eur, btc...
}

impl AssetKind {
    fn display(&self) -> String {
        match self {
            AssetKind::CoinId(id) => id.clone(),
            AssetKind::VsCurrency(vs) => vs.to_uppercase(),
        }
    }
}

// 嚴格解析加密貨幣：alias → /search 精確（重名取市值靠前）→ 本地清單精確；不做模糊
async fn resolve_coin_id_strict(
    client: &reqwest::Client,
    coins: &[CoinInfo],
    s: &str,
) -> Result<String> {
    let q = s.trim().to_lowercase();
    if q.is_empty() {
        return Err(anyhow!("幣種不可為空"));
    }
    if let Some(id) = alias_coin_id(&q) {
        return Ok(id.to_string());
    }
    if let Some(id) = search_coin_id_online_exact(client, &q).await? {
        return Ok(id);
    }
    if let Some(c) = coins.iter().find(|c| c.id.eq_ignore_ascii_case(&q)) {
        return Ok(c.id.clone());
    }
    if let Some(c) = coins.iter().find(|c| c.symbol.eq_ignore_ascii_case(&q)) {
        return Ok(c.id.clone());
    }
    if let Some(c) = coins.iter().find(|c| c.name.to_lowercase() == q) {
        return Ok(c.id.clone());
    }
    Err(anyhow!(
        "無法識別幣種 '{}'. 請用 id/symbol/name（例：tia/celestia, sol/solana）",
        s
    ))
}

/// 嚴格：只接受精確等於；若 symbol 重名則選市值排名最前
async fn search_coin_id_online_exact(
    client: &reqwest::Client,
    query: &str,
) -> Result<Option<String>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(None);
    }
    let url = format!("{}/search?query={}", COINGECKO_BASE, urlencoding::encode(&q));
    let resp = request_with_retry(client, client.get(&url)).await?;

    #[derive(Deserialize)]
    struct SearchCoin {
        id: String,
        symbol: String,
        name: String,
        #[serde(default)]
        market_cap_rank: Option<i64>,
    }
    #[derive(Deserialize)]
    struct SearchResp {
        coins: Vec<SearchCoin>,
    }

    let sr: SearchResp = resp.json().await?;

    if let Some(c) = sr.coins.iter().find(|c| c.id.eq_ignore_ascii_case(&q)) {
        return Ok(Some(c.id.clone()));
    }
    if let Some(c) = sr.coins.iter().find(|c| c.name.to_lowercase() == q) {
        return Ok(Some(c.id.clone()));
    }

    let mut exact_symbol: Vec<&SearchCoin> =
        sr.coins.iter().filter(|c| c.symbol.eq_ignore_ascii_case(&q)).collect();
    if !exact_symbol.is_empty() {
        exact_symbol.sort_by_key(|c| c.market_cap_rank.unwrap_or(i64::MAX));
        return Ok(Some(exact_symbol[0].id.clone()));
    }

    Ok(None)
}

/// 幣價查詢：回傳 map[coin_id][vs] = price
async fn fetch_simple_price(
    client: &reqwest::Client,
    ids: &[&str],
    vs: &[String],
) -> Result<HashMap<String, HashMap<String, f64>>> {
    let url = format!(
        "{}/simple/price?ids={}&vs_currencies={}&include_last_updated_at=true",
        COINGECKO_BASE,
        urlencoding::encode(&ids.join(",")),
        urlencoding::encode(&vs.join(","))
    );

    let resp = request_with_retry(client, client.get(&url)).await?;
    let map: HashMap<String, serde_json::Value> = resp.json().await?;

    let mut out: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for (coin_id, v) in map {
        if let Some(obj) = v.as_object() {
            let mut inner = HashMap::new();
            for (k, vv) in obj {
                if k == "last_updated_at" {
                    continue;
                }
                if let Some(f) = vv.as_f64() {
                    inner.insert(k.to_lowercase(), f);
                }
            }
            out.insert(coin_id, inner);
        }
    }
    Ok(out)
}

/// 兩點換算：from_kind -> to_kind，回傳折算後金額
async fn convert_pair(
    client: &reqwest::Client,
    from_kind: &AssetKind,
    to_kind: &AssetKind,
    amount: f64,
) -> Result<f64> {
    match (from_kind, to_kind) {
        // 幣→幣：以 USD 作樞紐： rate = price(from,USD)/price(to,USD)
        (AssetKind::CoinId(from_id), AssetKind::CoinId(to_id)) => {
            let vs = vec!["usd".to_string()];
            let prices = fetch_simple_price(client, &[from_id.as_str(), to_id.as_str()], &vs).await?;
            let pf = *prices
                .get(from_id)
                .and_then(|m| m.get("usd"))
                .ok_or_else(|| anyhow!("查不到 {} 的 USD 價格", from_id))?;
            let pt = *prices
                .get(to_id)
                .and_then(|m| m.get("usd"))
                .ok_or_else(|| anyhow!("查不到 {} 的 USD 價格", to_id))?;
            Ok(amount * (pf / pt))
        }
        // 幣→法幣：直接拿 price(from, to_vs)
        (AssetKind::CoinId(from_id), AssetKind::VsCurrency(to_vs)) => {
            let prices = fetch_simple_price(client, &[from_id.as_str()], &[to_vs.clone()]).await?;
            let p = *prices
                .get(from_id)
                .and_then(|m| m.get(to_vs.as_str()))
                .ok_or_else(|| anyhow!("查不到 {} 對 {} 的價格", from_id, to_vs))?;
            Ok(amount * p)
        }
        // 法幣→幣：amount / price(coin, vs)
        (AssetKind::VsCurrency(from_vs), AssetKind::CoinId(to_id)) => {
            let prices = fetch_simple_price(client, &[to_id.as_str()], &[from_vs.clone()]).await?;
            let p = *prices
                .get(to_id)
                .and_then(|m| m.get(from_vs.as_str()))
                .ok_or_else(|| anyhow!("查不到 {} 對 {} 的價格", to_id, from_vs))?;
            Ok(amount / p)
        }
        // 法幣→法幣：用 BTC 當橋接幣： rate = BTC_to / BTC_from
        (AssetKind::VsCurrency(from_vs), AssetKind::VsCurrency(to_vs)) => {
            let bridge = "bitcoin";
            let prices =
                fetch_simple_price(client, &[bridge], &[from_vs.clone(), to_vs.clone()]).await?;
            let m = prices
                .get(bridge)
                .ok_or_else(|| anyhow!("查不到 BTC 的法幣價格"))?;

            let k_from = from_vs.to_lowercase();
            let k_to = to_vs.to_lowercase();

            let p_from = *m
                .get(k_from.as_str())
                .ok_or_else(|| anyhow!("查不到 BTC 對 {} 的價格", from_vs))?;
            let p_to = *m
                .get(k_to.as_str())
                .ok_or_else(|| anyhow!("查不到 BTC 對 {} 的價格", to_vs))?;
            Ok(amount * (p_to / p_from))
        }
    }
}

async fn request_with_retry(
    _client: &reqwest::Client, // 消除未使用參數警告
    req: reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let mut tries = 0usize;
    loop {
        let builder = req
            .try_clone()
            .ok_or_else(|| anyhow!("request 無法複製（try_clone 失敗）"))?;
        match builder.send().await {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) if r.status().as_u16() == 429 => {
                let wait = retry_after_delay(r.headers());
                sleep(wait).await;
            }
            Ok(r) => {
                let code = r.status();
                let text = r.text().await.unwrap_or_default();
                return Err(anyhow!("HTTP {}: {}", code, text));
            }
            Err(e) => {
                if tries >= 4 {
                    return Err(anyhow!("請求失敗（多次重試後仍失敗）：{}", e));
                }
                // 指數退避（無外部依賴）
                let backoff = [400u64, 800, 1600, 3200, 6400][tries.min(4)];
                sleep(Duration::from_millis(backoff)).await;
            }
        }
        tries += 1;
    }
}

fn retry_after_delay(headers: &HeaderMap) -> Duration {
    if let Some(v) = headers.get(RETRY_AFTER) {
        if let Ok(s) = v.to_str() {
            if let Ok(secs) = s.parse::<u64>() {
                return Duration::from_secs(secs.min(15));
            }
        }
    }
    Duration::from_secs(2)
}

fn print_result(
    coin_id: &str,
    amount: f64,
    vs: &[String],
    prices: &HashMap<String, HashMap<String, f64>>,
) -> Result<()> {
    let Some(map) = prices.get(coin_id) else {
        return Err(anyhow!("查不到 '{}' 的最新價格，請稍後再試", coin_id));
    };

    println!("幣種: {} | 數量: {}", coin_id, amount);
    for cur in vs {
        if let Some(p) = map.get(cur) {
            let v = amount * p;
            println!("  = {:>12.6} {}", v, cur.to_uppercase());
        } else {
            println!("  (無 {} 報價)", cur.to_uppercase());
        }
    }
    Ok(())
}
