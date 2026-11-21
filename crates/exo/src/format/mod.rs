//! The rules that turn mail into the text Exo shows: dates, names, and the width estimates and
//! truncation that decide what fits in a row. Pure functions, tested on their own.

pub mod dates;
pub mod failure;
pub mod friendly;
mod lists;
pub(crate) mod measure;
pub mod names;
pub mod row;
pub mod urls;
mod widths;

/// `count` with its thousands grouped in English: `1,240`.
pub fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1240), "1,240");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
