#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_routes_compiles() {
        let _routes = build_routes("test-key".to_string());
        // Just ensure building routes doesn't panic
    }
}
