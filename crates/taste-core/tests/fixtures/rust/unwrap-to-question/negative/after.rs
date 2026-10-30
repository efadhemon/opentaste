pub fn bump(counter: &Mutex<u64>) -> Result<u64, Error> {
    let mut value = counter.lock()?;
    *value += 1;
    Ok(*value)
}
