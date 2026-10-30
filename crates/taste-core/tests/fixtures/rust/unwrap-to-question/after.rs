pub fn load(path: &Path) -> Result<Config, Error> {
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
