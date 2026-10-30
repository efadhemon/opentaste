pub fn bump(counter: &Mutex<u64>) -> Result<u64, Error> {
    let mut value = counter.lock().unwrap();
    *value += 1;
    Ok(*value)
}
