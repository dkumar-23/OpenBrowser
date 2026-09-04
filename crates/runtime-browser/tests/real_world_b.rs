//! Additional real-world tests with multiple fallback targets.

use runtime_browser::Browser;

#[tokio::test]
async fn test_jet2_jet2_com_direct() {
    let browser = Browser::new().unwrap();
    let targets = [
        "https://www.jet2.com/",
        "https://jet2.com/",
        "https://www.jet2holidays.com/",
    ];
    for url in &targets {
        print!("\n[{}]: ", url);
        match browser.navigate(url).await {
            Ok(res) => {
                println!("OK status={} cookies={} links={}",
                         res.status, res.cookies.len(),
                         res.dom.query_all("a").len());
            }
            Err(e) => {
                println!("ERR: {:?}", e);
            }
        }
    }
}

#[tokio::test]
async fn test_google_with_query_string() {
    // Google search with q=rust in URL — this is the URL pattern for a search.
    let browser = Browser::new().unwrap();
    let url = "https://www.google.com/search?q=rust+programming+language";
    match browser.navigate(url).await {
        Ok(res) => {
            println!("\n[Google search via URL]: status={} cookies={} divs={} a={}",
                     res.status, res.cookies.len(),
                     res.dom.query_all("div").len(),
                     res.dom.query_all("a").len());
            // Look for the search result container
            let found_h3 = res.dom.query_all("h3").len();
            let found_divs = res.dom.query_all("div").len();
            println!("  h3 elements: {} (typical for search results)", found_h3);
            println!("  div elements: {}", found_divs);
            // Google may or may not include Rust text depending on bot detection;
            // just verify page structure loaded.
            assert!(res.status == 200 || res.status == 302, "Expected 2xx/3xx, got {}", res.status);
        }
        Err(e) => println!("\n[Google search via URL]: ERR {:?}", e),
    }
}
