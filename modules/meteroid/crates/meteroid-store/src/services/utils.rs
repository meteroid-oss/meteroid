use chrono::Datelike;
use chrono::NaiveDate;

pub fn format_invoice_number(number: i64, format: String, date: NaiveDate) -> String {
    format
        .replace("{number}", &number.to_string())
        .replace("{YYYY}", &date.year().to_string())
        .replace("{MM}", &date.month().to_string())
        .replace("{DD}", &date.day().to_string())
}
