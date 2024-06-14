use da_client_interface::DaConfig;
use utils::env_utils::{get_env_var_or_default, get_env_var_or_panic};

#[derive(Clone, Debug)]
pub struct ZgDaConfig {
    pub url: String,
    pub disperser_retry_delay_ms: u32,
    pub status_retry_delay_ms: u32,
}

impl DaConfig for ZgDaConfig {
    fn new_from_env() -> Self {
        let disperser_retry_delay_ms = get_env_var_or_default("DISPERSER_RETRY_DELAY_MS", "1000")
            .parse::<u32>()
            .expect("DISPERSER_RETRY_DELAY_MS valid");
        let status_retry_delay_ms = get_env_var_or_default("STATUS_RETRY_DELAY_MS", "1000")
            .parse::<u32>()
            .expect("DISPERSER_RETRY_DELAY_MS valid");

        Self { url: get_env_var_or_panic("ZG_DA_URL"), disperser_retry_delay_ms, status_retry_delay_ms }
    }
}
