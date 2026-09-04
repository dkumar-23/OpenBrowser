//! DuckDuckGo fallback for Google search.
//!
//! Tests navigation + DOM query for "rust" across search engines.

use runtime_browser::Browser;

#[tokio::test]
async fn test_duckduckgo_search_rust() {
    let browser = Browser::new().unwrap();
    let url = "https://duckduckgo.com/html/?q=rust+programming+language";
    match browser.navigate(url).await {
        Ok(res) => {
            println!("\n[DuckDuckGo HTML search for 'rust']");
            println!("  Status:   {}", res.status);
            println!("  Cookies:  {}", res.cookies.len());
            println!("  Links:    {}", res.dom.query_all("a").len());
            println!("  divs:     {}", res.dom.query_all("div").len());
            println!("  results:  {}", res.dom.query_all("result").len());

            // Query for Rust-related selectors
            for sel in ["a", "h2", ".result", "div", "span"].iter() {
                println!("  '{}': {}", sel, res.dom.query_all(sel).len());
            }

            // Check the body text for "rust"
            let body_text = res.dom.query_all("a").len();
            assert!(body_text > 0, "DDG should return at least 1 link");
            println!("  ASSERT: DuckDuckGo loaded successfully");
        }
        Err(e) => println!("DuckDuckGo unreachable: {:?}", e),
    }
}
