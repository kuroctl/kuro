//! Binaire de l'orchestrateur, sans extension. Le programme vit dans `kuro_api::app`, partagé
//! avec les binaires qui compilent Kuro avec des extensions (ADR-0051).

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    kuro_api::app::main(Vec::new()).await
}
