#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub struct OrderByParam {
    pub column: String,
    pub direction: OrderDirection,
}

impl OrderByParam {
    /// Parses an order_by string like "column.asc" or "column.desc".
    /// Falls back to the default if input is None or direction is missing.
    pub fn parse(input: Option<&str>, default: &str) -> Self {
        let raw = input.unwrap_or(default);
        let (column, direction) = match raw.rsplit_once('.') {
            Some((col, "asc")) => (col, OrderDirection::Asc),
            Some((col, "desc")) => (col, OrderDirection::Desc),
            _ => (raw, OrderDirection::Desc),
        };
        Self {
            column: column.to_string(),
            direction,
        }
    }

    /// Parses and validates an order_by string against allowed columns.
    /// Returns an error if the column is not in `allowed_columns` or the direction is invalid.
    pub fn parse_validated(
        input: Option<&str>,
        default: &str,
        allowed_columns: &[&str],
    ) -> Result<Self, String> {
        let raw = input.unwrap_or(default);
        let (column, direction) = match raw.rsplit_once('.') {
            Some((col, "asc")) => (col, OrderDirection::Asc),
            Some((col, "desc")) => (col, OrderDirection::Desc),
            Some((_, dir)) => {
                return Err(format!(
                    "Invalid sort direction '{dir}'. Must be 'asc' or 'desc'"
                ));
            }
            None => (raw, OrderDirection::Desc),
        };

        if !allowed_columns.contains(&column) {
            return Err(format!(
                "Invalid sort column '{column}'. Allowed values: {}",
                allowed_columns.join(", ")
            ));
        }

        Ok(Self {
            column: column.to_string(),
            direction,
        })
    }
}
