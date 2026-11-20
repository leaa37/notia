//! Exportar los movimientos de Finanzas a una planilla.
//!
//! `xlsx` es un libro de Excel con una hoja (importes como números, para
//! sumar y filtrar); `csv` separa con `;` y usa coma decimal, como lo abre
//! Excel en Argentina, con BOM para que respete los acentos.

use std::io::Write as _;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::finance::{read, valid_iso_date, FinanceCommandError, FinanceCommandResult, FinanceContext};
use crate::finance_screen::MovementRow;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPayload {
    pub context: FinanceContext,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// `xlsx` (the default) or `csv`.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFile {
    pub file_name: String,
    pub media_type: &'static str,
    pub base64: String,
    pub rows: usize,
}

const HEADERS: [&str; 8] = ["Fecha", "Tipo", "Nombre", "Categoría", "Cuenta", "Detalle", "Importe", "Moneda"];
/// Column of the amount, the only number.
const AMOUNT_COLUMN: usize = 6;

fn cells(row: &MovementRow) -> [String; 8] {
    [
        day_text(&row.date),
        (if row.kind == "income" { "Ingreso" } else { "Gasto" }).to_string(),
        row.name.clone(),
        row.category.clone().unwrap_or_default(),
        row.account.clone().unwrap_or_default(),
        row.detail.clone().unwrap_or_default(),
        row.amount.clone(),
        row.currency.clone(),
    ]
}

/// `2026-11-02` as `02/11/2026`.
fn day_text(date: &str) -> String {
    match (date.get(..4), date.get(5..7), date.get(8..10)) {
        (Some(year), Some(month), Some(day)) => format!("{day}/{month}/{year}"),
        _ => date.to_string(),
    }
}

pub fn finance_export(app: crate::host::AppHandle, payload: ExportPayload) -> FinanceCommandResult<ExportFile> {
    for date in [payload.from.as_deref(), payload.to.as_deref()].into_iter().flatten() {
        if !valid_iso_date(date) {
            return Err(FinanceCommandError::invalid("Las fechas de la exportación tienen que ser días válidos."));
        }
    }
    let rows = read(&app, &payload.context, |connection| {
        crate::finance_screen::rows_between(connection, payload.from.as_deref().unwrap_or("0000-01-01"), payload.to.as_deref().unwrap_or("9999-12-31"))
    })?;
    let name = match (payload.from.as_deref(), payload.to.as_deref()) {
        (None, None) => "finanzas-historial".to_string(),
        (from, to) => format!("finanzas-{}-a-{}", from.unwrap_or("inicio"), to.unwrap_or("hoy")),
    };
    let (extension, media_type, bytes) = match payload.format.as_deref().unwrap_or("xlsx") {
        "csv" => ("csv", "text/csv", csv(&rows).into_bytes()),
        "xlsx" => ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", xlsx(&rows).map_err(FinanceCommandError::storage)?),
        _ => return Err(FinanceCommandError::invalid("El formato tiene que ser xlsx o csv.")),
    };
    Ok(ExportFile {
        file_name: format!("{name}.{extension}"),
        media_type,
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        rows: rows.len(),
    })
}

fn csv_cell(value: &str) -> String {
    if value.contains([';', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub(crate) fn csv(rows: &[MovementRow]) -> String {
    let mut text = String::from("\u{feff}");
    text.push_str(&HEADERS.join(";"));
    text.push_str("\r\n");
    for row in rows {
        let mut values = cells(row);
        values[AMOUNT_COLUMN] = values[AMOUNT_COLUMN].replace('.', ",");
        text.push_str(&values.iter().map(|value| csv_cell(value)).collect::<Vec<_>>().join(";"));
        text.push_str("\r\n");
    }
    text
}

fn xml_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn column_name(index: usize) -> String {
    let mut name = String::new();
    let mut number = index + 1;
    while number > 0 {
        let rest = (number - 1) % 26;
        name.insert(0, char::from(b'A' + rest as u8));
        number = (number - 1) / 26;
    }
    name
}

fn sheet(rows: &[MovementRow]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols><col min="1" max="1" width="12" customWidth="1"/><col min="2" max="2" width="10" customWidth="1"/><col min="3" max="3" width="36" customWidth="1"/><col min="4" max="4" width="20" customWidth="1"/><col min="5" max="5" width="16" customWidth="1"/><col min="6" max="6" width="24" customWidth="1"/><col min="7" max="7" width="14" customWidth="1"/><col min="8" max="8" width="9" customWidth="1"/></cols><sheetData>"#,
    );
    let header = HEADERS.iter().map(|header| header.to_string()).collect::<Vec<_>>();
    let mut push_row = |number: usize, values: &[String], bold: bool| {
        xml.push_str(&format!("<row r=\"{number}\">"));
        for (index, value) in values.iter().enumerate() {
            let reference = format!("{}{number}", column_name(index));
            let style = if bold { " s=\"1\"" } else if index == AMOUNT_COLUMN { " s=\"2\"" } else { "" };
            if index == AMOUNT_COLUMN && !bold && value.parse::<f64>().is_ok() {
                xml.push_str(&format!("<c r=\"{reference}\"{style}><v>{value}</v></c>"));
            } else if !value.is_empty() {
                xml.push_str(&format!("<c r=\"{reference}\"{style} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", xml_text(value)));
            }
        }
        xml.push_str("</row>");
    };
    push_row(1, &header, true);
    for (index, row) in rows.iter().enumerate() {
        push_row(index + 2, &cells(row), false);
    }
    xml.push_str("</sheetData>");
    if !rows.is_empty() {
        xml.push_str(&format!("<autoFilter ref=\"A1:{}{}\"/>", column_name(HEADERS.len() - 1), rows.len() + 1));
    }
    xml.push_str("</worksheet>");
    xml
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Movimientos" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const WORKBOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
/// Style 1: bold header; style 2: an amount with thousands and two decimals.
const STYLES: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="#,##0.00"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"##;

pub(crate) fn xlsx(rows: &[MovementRow]) -> Result<Vec<u8>, String> {
    let failed = |error: std::io::Error| format!("No se pudo armar la planilla: {error}");
    let zipped = |error: zip::result::ZipError| format!("No se pudo armar la planilla: {error}");
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in [
            ("[Content_Types].xml", CONTENT_TYPES.to_string()),
            ("_rels/.rels", ROOT_RELS.to_string()),
            ("xl/workbook.xml", WORKBOOK.to_string()),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS.to_string()),
            ("xl/styles.xml", STYLES.to_string()),
            ("xl/worksheets/sheet1.xml", sheet(rows)),
        ] {
            zip.start_file(path, options).map_err(zipped)?;
            zip.write_all(content.as_bytes()).map_err(failed)?;
        }
        zip.finish().map_err(zipped)?;
    }
    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, amount: &str) -> MovementRow {
        MovementRow {
            id: "m".into(),
            kind: "expense".into(),
            name: name.into(),
            category_id: Some("c".into()),
            category: Some("Supermercado".into()),
            account: Some("Banco".into()),
            amount: amount.into(),
            currency: "ARS".into(),
            date: "2026-11-02".into(),
            detail: Some("Cuota 1/3 · Visa".into()),
            tag: Some("card"),
        }
    }

    #[test]
    fn a_csv_reads_as_excel_expects_it_in_argentina() {
        let text = csv(&[row("Coto; sucursal \"centro\"", "15000.50")]);
        assert!(text.starts_with('\u{feff}'), "with a BOM for the accents");
        let lines = text.trim_start_matches('\u{feff}').split("\r\n").collect::<Vec<_>>();
        assert_eq!(lines[0], HEADERS.join(";"));
        assert_eq!(lines[1], "02/11/2026;Gasto;\"Coto; sucursal \"\"centro\"\"\";Supermercado;Banco;Cuota 1/3 · Visa;15000,50;ARS");
    }

    #[test]
    fn a_workbook_has_its_parts_and_numbers_as_numbers() {
        let bytes = xlsx(&[row("Leche & pan <2>", "1200.5")]).expect("xlsx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        for part in ["[Content_Types].xml", "_rels/.rels", "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/styles.xml", "xl/worksheets/sheet1.xml"] {
            assert!(archive.by_name(part).is_ok(), "{part}");
        }
        let mut sheet = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("xl/worksheets/sheet1.xml").expect("sheet"), &mut sheet).expect("read");
        assert!(sheet.contains("<c r=\"G2\" s=\"2\"><v>1200.5</v></c>"), "{sheet}");
        assert!(sheet.contains("Leche &amp; pan &lt;2&gt;"), "text is escaped");
        assert!(sheet.contains("<autoFilter ref=\"A1:H2\"/>"));
    }

    #[test]
    fn columns_are_named_as_in_a_spreadsheet() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(10), "K");
        assert_eq!(column_name(26), "AA");
    }
}
