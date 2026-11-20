use std::time::Duration;

use serde::Serialize;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DOLAR_API_URL: &str = "https://dolarapi.com/v1/dolares";
const INFLATION_MONTHLY_URL: &str =
    "https://api.argentinadatos.com/v1/finanzas/indices/inflacion";
const INFLATION_ANNUAL_URL: &str =
    "https://api.argentinadatos.com/v1/finanzas/indices/inflacionInteranual";
const DOLLAR_HISTORY_URL: &str =
    "https://api.argentinadatos.com/v1/cotizaciones/dolares/oficial";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DollarQuote {
    pub kind: String,
    pub name: String,
    pub buy: f64,
    pub sell: f64,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct InflationIndex {
    pub period: String,
    pub percent: f64,
}

#[derive(Debug, Serialize)]
pub struct InflationIndices {
    pub monthly: Vec<InflationIndex>,
    pub annual: Vec<InflationIndex>,
}

#[derive(Debug, Serialize)]
pub struct HistoricalDollarQuote {
    pub date: String,
    pub buy: f64,
    pub sell: f64,
}

async fn fetch_json(url: &'static str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| "No se pudo preparar el proveedor financiero externo.".to_string())?;
    let response = tokio::time::timeout(
        REQUEST_TIMEOUT,
        client.get(url).header(reqwest::header::ACCEPT, "application/json").send(),
    )
    .await
    .map_err(|_| "El proveedor financiero externo agotó el tiempo de espera.".to_string())?
    .map_err(|_| "El proveedor financiero externo no está disponible.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "El proveedor financiero externo respondió HTTP {}.",
            response.status().as_u16()
        ));
    }
    tokio::time::timeout(REQUEST_TIMEOUT, response.json::<serde_json::Value>())
        .await
        .map_err(|_| "El proveedor financiero externo agotó el tiempo de espera.".to_string())?
        .map_err(|_| "El proveedor financiero externo devolvió JSON inválido.".to_string())
}

pub async fn dollar_quotes() -> Result<Vec<DollarQuote>, String> {
    let payload = fetch_json(DOLAR_API_URL).await?;
    let values = payload
        .as_array()
        .ok_or_else(|| "DolarApi devolvió un formato inesperado.".to_string())?;
    ["oficial", "blue", "tarjeta"]
        .into_iter()
        .map(|kind| {
            let value = values
                .iter()
                .find(|value| value.get("casa").and_then(serde_json::Value::as_str) == Some(kind))
                .ok_or_else(|| format!("DolarApi no devolvió la cotización {kind}."))?;
            let buy = finite_number(value, "compra")?;
            let sell = finite_number(value, "venta")?;
            let name = value
                .get("nombre")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "DolarApi no devolvió el nombre de la cotización.".to_string())?;
            let updated_at = value
                .get("fechaActualizacion")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "DolarApi no devolvió la fecha de actualización.".to_string())?;
            Ok(DollarQuote {
                kind: kind.to_string(),
                name: name.to_string(),
                buy,
                sell,
                updated_at: updated_at.to_string(),
            })
        })
        .collect()
}

pub async fn inflation_indices() -> Result<InflationIndices, String> {
    let monthly = inflation_series(INFLATION_MONTHLY_URL).await?;
    let annual = inflation_series(INFLATION_ANNUAL_URL).await?;
    Ok(InflationIndices { monthly, annual })
}

async fn inflation_series(url: &'static str) -> Result<Vec<InflationIndex>, String> {
    let payload = fetch_json(url).await?;
    let values = payload
        .as_array()
        .ok_or_else(|| "ArgentinaDatos devolvió un formato inesperado.".to_string())?;
    let mut result = values
        .iter()
        .filter_map(|value| {
            let date = value.get("fecha").and_then(serde_json::Value::as_str)?;
            if !date.is_ascii() || date.len() < 7 {
                return None;
            }
            let percent = finite_number(value, "valor").ok()?;
            let period = date.get(..7)?.to_string();
            (is_period(&period)).then_some(InflationIndex { period, percent })
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.period.cmp(&right.period));
    Ok(result)
}

pub async fn historical_dollar_quotes(
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<HistoricalDollarQuote>, String> {
    let payload = fetch_json(DOLLAR_HISTORY_URL).await?;
    let values = payload
        .as_array()
        .ok_or_else(|| "ArgentinaDatos devolvió un formato inesperado.".to_string())?;
    let mut result = values
        .iter()
        .filter_map(|value| {
            let raw_date = value.get("fecha").and_then(serde_json::Value::as_str)?;
            if !raw_date.is_ascii() || raw_date.len() < 10 {
                return None;
            }
            let date = raw_date.get(..10)?.to_string();
            let buy = finite_number(value, "compra").ok()?;
            let sell = finite_number(value, "venta").ok()?;
            (is_date(&date) && sell > 0.0).then_some(HistoricalDollarQuote { date, buy, sell })
        })
        .filter(|quote| from.is_none_or(|value| quote.date.as_str() >= value))
        .filter(|quote| to.is_none_or(|value| quote.date.as_str() <= value))
        .collect::<Vec<_>>();
    result.sort_by(|left, right| left.date.cmp(&right.date));
    Ok(result)
}

fn finite_number(value: &serde_json::Value, key: &str) -> Result<f64, String> {
    let number = value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| format!("El proveedor no devolvió un número válido para {key}."))?;
    number
        .is_finite()
        .then_some(number)
        .ok_or_else(|| format!("El proveedor devolvió un número inválido para {key}."))
}

fn is_period(value: &str) -> bool {
    value.is_ascii()
        && value.len() == 7
        && value.as_bytes().get(4) == Some(&b'-')
        && value[0..4].parse::<u16>().is_ok()
        && (1..=12).contains(&value[5..7].parse::<u8>().unwrap_or_default())
}

fn is_date(value: &str) -> bool {
    value.is_ascii()
        && value.len() == 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value[0..4].parse::<u16>().is_ok()
        && (1..=12).contains(&value[5..7].parse::<u8>().unwrap_or_default())
        && (1..=31).contains(&value[8..10].parse::<u8>().unwrap_or_default())
}

pub async fn finance_dollar_quotes() -> Result<Vec<DollarQuote>, String> {
    dollar_quotes().await
}

