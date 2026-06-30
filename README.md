# Coinex

Coinex is a command-line cryptocurrency conversion tool powered by the [CoinGecko API](https://www.coingecko.com/en/api). It can convert in real time between:

- Cryptocurrency → fiat / quote currency, for example `bitcoin` → `twd`
- Fiat / quote currency → cryptocurrency, for example `usd` → `solana`
- Cryptocurrency → cryptocurrency, for example `eth` → `btc`
- Fiat / quote currency → fiat / quote currency, for example `usd` → `twd`

> The package name in `Cargo.toml` is `coinex`; the CLI display name is `CoinEx`.

## Features

- Resolves CoinGecko coin `id`, `symbol`, or `name`.
- Includes aliases for common coins to reduce ambiguous-symbol surprises, such as `btc`, `eth`, `sol`, and `tia`.
- Downloads and caches the CoinGecko coin list automatically; the default cache TTL is `1d`.
- Supports `--refresh` to force a fresh coin-list download.
- Supports the `COINGECKO_API_KEY` environment variable for CoinGecko Demo API keys.
- Retries HTTP 429 responses and transient network failures with backoff.

## Installation and Build

Install the Rust toolchain first, then run this from the repository root:

```bash
cargo build --release
```

The release binary is created at:

```bash
./target/release/coinex
```

During development, you can run the CLI directly with Cargo:

```bash
cargo run -- [OPTIONS] <AMOUNT>
```

## Quick Start

### Convert cryptocurrency to fiat

```bash
cargo run -- --crypto bitcoin 0.01 --to-fiat twd
```

### Convert fiat to cryptocurrency

```bash
cargo run -- --fiat usd 100 --to-crypto sol
```

### Convert one cryptocurrency to another

```bash
cargo run -- --crypto eth 1.5 --to-crypto btc
```

### Convert one fiat / quote currency to another

```bash
cargo run -- --fiat usd 100 --to-fiat twd
```

## Usage

```text
Usage: coinex [OPTIONS] <--crypto <COIN>|--fiat <VS_CURRENCY>> <--to-crypto <COIN>|--to-fiat <VS_CURRENCY>> <AMOUNT>
```

Each conversion must specify:

1. One source type: either `--crypto` or `--fiat`.
2. The amount to convert: `<AMOUNT>`.
3. One destination type: either `--to-crypto` or `--to-fiat`.

## Options

| Option | Description |
| --- | --- |
| `--crypto <COIN>` | Use a cryptocurrency as the source. Accepts a CoinGecko coin id, symbol, or name, such as `bitcoin`, `btc`, `solana`, or `sol`. |
| `--fiat <VS_CURRENCY>` | Use a fiat or CoinGecko quote currency as the source, such as `usd`, `eur`, `twd`, `btc`, or `eth`. |
| `<AMOUNT>` | Amount to convert. Decimals are supported, such as `0.01`, `100`, or `1.5`. |
| `--to-crypto <COIN>` | Use a cryptocurrency as the destination. Accepts a CoinGecko coin id, symbol, or name. |
| `--to-fiat <VS_CURRENCY>` | Use a fiat or CoinGecko quote currency as the destination, such as `usd`, `twd`, or `btc`. |
| `--refresh` | Ignore the local coin-list cache and force a fresh download from CoinGecko. |
| `--cache-ttl <DURATION>` | Set the coin-list cache time-to-live using humantime syntax, such as `12h`, `1d`, or `30m`. Defaults to `1d`. |
| `-h`, `--help` | Print help. |
| `-V`, `--version` | Print version information. |

## Conversion Rules

- Coin → fiat / quote currency: fetches the source coin price in the destination quote currency directly.
- Fiat / quote currency → coin: fetches the destination coin price in the source quote currency and converts inversely.
- Coin → coin: uses USD as the bridge quote currency.
- Fiat / quote currency → fiat / quote currency: uses Bitcoin as the bridge asset.

## Cache Location

Coinex stores the CoinGecko coin-list cache in `CoinEx/coins.json` under the system cache directory.

Common examples:

- Linux: `~/.cache/CoinEx/coins.json`
- macOS: `~/Library/Caches/CoinEx/coins.json`
- Windows: `%LOCALAPPDATA%/CoinEx/coins.json`

## CoinGecko API Key

If you have a CoinGecko Demo API key, set it with:

```bash
export COINGECKO_API_KEY="your API key"
```

Coinex sends the value as the `x-cg-demo-api-key` request header.

## FAQ

### Why can't `--crypto` and `--fiat` be used together?

A conversion can only have one source, so `--crypto` and `--fiat` are mutually exclusive and one of them is required.

### Why can't `--to-crypto` and `--to-fiat` be used together?

A conversion can only have one destination, so `--to-crypto` and `--to-fiat` are mutually exclusive and one of them is required.

### What should I do if a coin cannot be resolved?

Prefer the CoinGecko coin id, such as `bitcoin`, `ethereum`, `solana`, or `celestia`. If a symbol is shared by multiple coins, Coinex prefers the CoinGecko search result with the better market-cap rank.

## Development Check

```bash
cargo check
```
