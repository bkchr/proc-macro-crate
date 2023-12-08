use std::collections::HashMap;

pub enum Toml {
    Nested(HashMap<String, Box<Toml>>),
    Value(String),
    Empty,
}

fn parse_toml(toml_str: String) -> Toml {
    let mut toml = Toml::Empty;

    for line in toml_str.lines() {
        let line = line.trim();

        match line.chars().first() {
            // Ignore comments
            "#" => continue,
            "[" => {

            }
        }
    }
}
