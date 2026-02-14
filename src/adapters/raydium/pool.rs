pub fn pool_open_time(data: &[u8]) -> Option<u64> {
    let pool_open_time_bytes: [u8; 8] = data.get(373..381)?.try_into().ok()?;
    Some(u64::from_le_bytes(pool_open_time_bytes))
}
