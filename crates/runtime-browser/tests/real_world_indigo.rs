//! Fallback for jet2.com — test indigo.com and duckduckgo.
//!
//! Real-world practical: if direct site unreachable, use DDG for info.

use runtime_browser::Browser;

#[tokio::test]
async fn test_jet2_fallback_indigo_duck() {
    let browser = Browser::new().unwrap();

    // Try direct first
    println!("=== Direct: jet2.com ===");
    match browser.navigate("https://www.jet2.com/").await {
        Ok(res) => println!("  Direct OK: status={}", res.status),
        Err(_) => println!("  Direct FAIL: unreachable"),
    }

    // Fallback: find jet2 via DDG
    println!("=== Fallback: DDG search 'jet2 flights' ===");
    let url = "https://duckduckgo.com/html/?q=jet2+flights";
    match browser.navigate(url).await {
        Ok(res) => {
            println!("  DDG OK: status={}", res.status);
            println!("  Links: {}, Divs: {}, h2: {}",
                     res.dom.query_all("a").len(),
                     res.dom.query_all("div").len(),
                     res.dom.query_all("h2").len());
        }
        Err(e) => println!("  DDG FAIL: {:?}", e),
    }

    // Extra target: indigo.com (Indian airline) as user requested
    println!("=== Direct: indigo.com ===");
    match browser.navigate("https://www.indigo.in/").await {
        Ok(res) => println!("  Indigo OK: status={}", res.status),
        Err(_) => println!("  Indigo FAIL: unreachable"),
    }
}
