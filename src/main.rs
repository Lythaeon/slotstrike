#[tokio::main]
async fn main() {
    slotstrike::app::bootstrap::run().await;
}
