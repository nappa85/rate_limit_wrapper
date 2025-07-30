# Async rate limit implementation

Async rate limit wrapper to easily manage resources

## Example

```rust
use std::time::Duration;
use rate_limit_wrapper::RateLimit;

#[tokio::main]
async fn main() {
  // 10 requests per second
  let rm = RateLimit::new(10, Duration::from_secs(1), reqwest::Client::new());

  // Access inner client bypassing rate limit
  let request = rm.as_ref().get("https://www.rust-lang.org").build().unwrap();

  // Apply rate limit
  let client = rm.rate_limit().await;
  client.execute(request).await.unwrap();
}
```
