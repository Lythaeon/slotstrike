use crate::domain::entities::SnipeRule;

#[trait_variant::make(Send + Sync)]
pub trait RuleRepository {
    async fn load_rules(
        &self,
        file_type: &str,
        initial: bool,
    ) -> Result<Vec<SnipeRule>, std::io::Error>;
}
