pub fn replace_first_and_last(s: &str, replacement: &str) -> String {
    if s.len() <= 1 {
        return replacement.to_string() + replacement;
    }

    let middle = &s[1..s.len() - 1];

    format!("{}{}{}", replacement, middle, replacement)
}
