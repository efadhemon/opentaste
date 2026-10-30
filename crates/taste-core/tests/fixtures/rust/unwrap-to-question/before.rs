pub fn load(path: &Path) -> Result<Config, Error> {
    let text = fs::read_to_string(path).unwrap();
    Ok(toml::from_str(&text)?)
}
