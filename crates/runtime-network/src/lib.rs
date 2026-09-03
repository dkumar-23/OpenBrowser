pub struct HttpClient;
impl HttpClient { pub fn new() -> Self { Self } pub async fn get(&self, url: &str) -> anyhow::Result<String> { Ok(format!("stub response from {}", url)) } }
