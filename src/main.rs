//Long/Short oranı, 1'den büyükse long pozisyonların, 1'den küçükse short pozisyonların daha fazla olduğunu gösterir.
//RSI değeri 0 ile 100 arasındadır. Genellikle 30'un altı aşırı satım, 70'in üstü aşırı alım olarak değerlendirilir.
#![allow(dead_code)]

use futures::future::join_all;
use prettytable::{row, Table};
use reqwest;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug)]
struct CoinInfo {
    binance_price: String,
    coinbase_price: String,
    okx_price: String,
    funding_rate: String,
    price_change_24h: String,
    long_short_ratio: String,
    rsi: String,
}

async fn fetch_price(
    client: &reqwest::Client,
    url: String,
) -> Result<Value, Box<dyn std::error::Error>> {
    let resp = client.get(&url).send().await?;
    let body: Value = resp.json().await?;
    Ok(body)
}

async fn get_binance_prices(
    client: &reqwest::Client,
    symbols: &[&str],
) -> HashMap<String, CoinInfo> {
    let futures = symbols.iter().map(|&symbol| {
        let price_url = format!("https://api.binance.com/api/v3/ticker/price?symbol={}USDT", symbol);
        let funding_url = format!("https://fapi.binance.com/fapi/v1/premiumIndex?symbol={}USDT", symbol);
        let ticker_url = format!("https://api.binance.com/api/v3/ticker/24hr?symbol={}USDT", symbol);
        let long_short_url = format!("https://fapi.binance.com/futures/data/globalLongShortAccountRatio?symbol={}USDT&period=5m", symbol);
        let klines_url = format!("https://api.binance.com/api/v3/klines?symbol={}USDT&interval=1h&limit=14", symbol);    
        async move {
            let price_result = fetch_price(client, price_url).await;
            let funding_result = fetch_price(client, funding_url).await;
            let ticker_result = fetch_price(client, ticker_url).await;
            let long_short_result = fetch_price(client, long_short_url).await;
            let klines_result = fetch_price(client, klines_url).await;
            (symbol, price_result, funding_result, ticker_result, long_short_result, klines_result)
        }
    });

    let results = join_all(futures).await;
    let mut prices = HashMap::new();

    for (symbol, price_result, funding_result, ticker_result, long_short_result, klines_result) in
        results
    {
        let mut coin_info = CoinInfo {
            binance_price: "N/A".to_string(),
            coinbase_price: "N/A".to_string(),
            okx_price: "N/A".to_string(),
            funding_rate: "N/A".to_string(),
            price_change_24h: "N/A".to_string(),
            long_short_ratio: "N/A".to_string(),
            rsi: "N/A".to_string(),
        };

        if let Ok(body) = price_result {
            if let Some(price) = body["price"].as_str() {
                coin_info.binance_price = price.to_string();
            }
        }

        if let Ok(body) = funding_result {
            if let Some(rate) = body["lastFundingRate"].as_str() {
                let rate_float: f64 = rate.parse().unwrap_or(0.0);
                coin_info.funding_rate = format!("{:.4}%", rate_float * 100.0);
            }
        }

        if let Ok(body) = ticker_result {
            if let Some(change) = body["priceChangePercent"].as_str() {
                coin_info.price_change_24h = format!("{}%", change);
            }
        }

        if let Ok(body) = long_short_result {
            if let Some(ratio) = body.as_array().and_then(|arr| arr.first()) {
                if let (Some(long), Some(short)) = (
                    ratio["longAccount"].as_str(),
                    ratio["shortAccount"].as_str(),
                ) {
                    let long_float: f64 = long.parse().unwrap_or(0.0);
                    let short_float: f64 = short.parse().unwrap_or(0.0);
                    let ratio = if short_float != 0.0 {
                        long_float / short_float
                    } else {
                        0.0
                    };
                    coin_info.long_short_ratio = format!("{:.2}", ratio);
                }
            }
        }

        if let Ok(body) = klines_result {
            if let Some(klines) = body.as_array() {
                let closes: Vec<f64> = klines
                    .iter()
                    .filter_map(|k| k[4].as_str())
                    .filter_map(|c| c.parse().ok())
                    .collect();
                if closes.len() == 14 {
                    let rsi = calculate_rsi(&closes);
                    coin_info.rsi = format!("{:.2}", rsi);
                }
            }
        }

        prices.insert(symbol.to_string(), coin_info);
    }

    prices
}

fn calculate_rsi(closes: &[f64]) -> f64 {
    let mut gains = Vec::new();
    let mut losses = Vec::new();
    for i in 1..closes.len() {
        let difference = closes[i] - closes[i - 1];
        if difference >= 0.0 {
            gains.push(difference);
            losses.push(0.0);
        } else {
            gains.push(0.0);
            losses.push(-difference);
        }
    }
    let average_gain: f64 = gains.iter().sum::<f64>() / 14.0;
    let average_loss: f64 = losses.iter().sum::<f64>() / 14.0;
    if average_loss == 0.0 {
        100.0
    } else {
        let rs = average_gain / average_loss;
        100.0 - (100.0 / (1.0 + rs))
    }
}

async fn get_coinbase_prices(
    client: &reqwest::Client,
    symbols: &[&str],
) -> HashMap<String, String> {
    let futures = symbols.iter().map(|&symbol| {
        let url = format!("https://api.coinbase.com/v2/prices/{}-USD/spot", symbol);
        fetch_price(client, url)
    });

    let results = join_all(futures).await;
    let mut prices = HashMap::new();

    for (result, &symbol) in results.into_iter().zip(symbols.iter()) {
        if let Ok(body) = result {
            if let Some(price) = body["data"]["amount"].as_str() {
                prices.insert(symbol.to_string(), price.to_string());
            }
        }
    }

    prices
}

async fn get_okx_prices(client: &reqwest::Client, symbols: &[&str]) -> HashMap<String, String> {
    let futures = symbols.iter().map(|&symbol| {
        let url = format!(
            "https://www.okx.com/api/v5/market/ticker?instId={}-USDT",
            symbol
        );
        fetch_price(client, url)
    });

    let results = join_all(futures).await;
    let mut prices = HashMap::new();

    for (result, &symbol) in results.into_iter().zip(symbols.iter()) {
        if let Ok(body) = result {
            if let Some(price) = body["data"][0]["last"].as_str() {
                prices.insert(symbol.to_string(), price.to_string());
            }
        }
    }

    prices
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let symbols = vec![
        "BTC", "ETH", "SOL", "DOT", "INJ", "STRK", "ARB", "POL", "SUI", "RENDER",
    ];

    let client = reqwest::Client::new();

    let binance_info = get_binance_prices(&client, &symbols).await;
    let coinbase_prices = get_coinbase_prices(&client, &symbols).await;
    let okx_prices = get_okx_prices(&client, &symbols).await;

    let mut table = Table::new();
    table.add_row(row![
        "Coin",
        "Binance 🔶",
        "Coinbase 🟦",
        "OKX 🔵",
        "Funding Rate",
        "24h Change",
        "Long/Short Ratio",
        "RSI"
    ]);

    for symbol in symbols {
        let coin_info = binance_info.get(symbol).unwrap();
        let coinbase_price = coinbase_prices
            .get(symbol)
            .map(String::as_str)
            .unwrap_or("N/A");
        let okx_price = okx_prices.get(symbol).map(String::as_str).unwrap_or("N/A");

        table.add_row(row![
            symbol,
            coin_info.binance_price,
            coinbase_price,
            okx_price,
            coin_info.funding_rate,
            coin_info.price_change_24h,
            coin_info.long_short_ratio,
            coin_info.rsi
        ]);
    }

    table.printstd();

    Ok(())
}
