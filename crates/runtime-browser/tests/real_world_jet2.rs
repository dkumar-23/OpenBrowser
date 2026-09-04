//! Real-world: jet2.com navigation with multiple fallbacks.

use runtime_browser::Browser;

#[tokio::test]
async fn test_jet2_with_fallbacks() {
    let browser = Browser::new().unwrap();

    let targets = [
        "https://www.jet2.com/",
        "https://jet2.com/",
        "https://www.jet2holidays.com/",
        "https://www.duckduckgo.com/?q=jet2",  // fallback
    ];

    for url in &targets {
        print!("\n[{}]: ", url);
        match browser.navigate(url).await {
            Ok(res) => {
                println!("OK status={} cookies={} links={} divs={}",
                         res.status, res.cookies.len(),
                         res.dom.query_all("a").len(),
                         res.dom.query_all("div").len());
            }
            Err(e) => {
                println!("ERR: {:?}", e);
            }
        }
    }
}
