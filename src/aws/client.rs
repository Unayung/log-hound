use anyhow::Result;
use aws_config::BehaviorVersion;
use aws_sdk_cloudwatchlogs::Client;

pub async fn create_client(profile: Option<&str>, region: Option<&str>) -> Result<Client> {
    let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

    if let Some(profile_name) = profile {
        config_loader = config_loader.profile_name(profile_name);
    }

    if let Some(region_name) = region {
        config_loader = config_loader.region(aws_config::Region::new(region_name.to_string()));
    }

    let config = config_loader.load().await;
    let client = Client::new(&config);

    Ok(client)
}
