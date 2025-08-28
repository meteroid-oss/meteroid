use chrono::Datelike;
use chrono::NaiveDate;

pub fn format_invoice_number(number: i64, format: String, date: NaiveDate) -> String {
    format
        .replace("{number}", &format!("{:04}", number))
        .replace("{YYYY}", &date.year().to_string())
        .replace("{MM}", &format!("{:02}", date.month()))
        .replace("{DD}", &format!("{:02}", date.day()))
}
