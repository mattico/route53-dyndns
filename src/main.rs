#[macro_use]
extern crate log;
extern crate env_logger;
extern crate reqwest;
extern crate rusoto_core;
extern crate rusoto_route53;

use rusoto_core::Region;
use rusoto_route53::{
    Change, ChangeBatch, ChangeResourceRecordSetsRequest, GetChangeRequest,
    ListHostedZonesByNameRequest, ListResourceRecordSetsRequest, ResourceRecord, ResourceRecordSet,
    Route53, Route53Client,
};
use std::error::Error;
use std::thread;
use std::time::{Duration, SystemTime};

fn get_env(var: &str) -> String {
    std::env::var(var).expect(&format!("{} not set", var))
}

fn main() {
    let env = env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info");
    env_logger::init_from_env(env);

    let mut dns_name = get_env("ROUTE53_DOMAIN_A_RECORD");
    if !dns_name.ends_with('.') {
        dns_name.push('.');
    }
    let my_ip_url = get_env("ROUTE53_IP_URL");
    let update_frequency = get_env("ROUTE53_UPDATE_FREQUENCY")
        .parse::<u64>()
        .expect("Can't parse ROUTE53_UPDATE_FREQUENCY as integer");

    loop {
        match run(&dns_name, &my_ip_url) {
            Ok(true) => info!("A Record Updated"),
            Ok(false) => info!("No update required"),
            Err(e) => error!("{:?}", e),
        }
        thread::sleep(Duration::from_secs(update_frequency));
    }
}

fn run(dns_name: &str, my_ip_url: &str) -> Result<bool, Box<dyn Error>> {
    let my_ip = reqwest::get(my_ip_url)?.text()?;
    info!("My IP: {}", &my_ip);

    info!("Domain: {}", &dns_name);
    let client = Route53Client::new(Region::UsEast1);
    let request = ListHostedZonesByNameRequest {
        dns_name: Some(dns_name.into()),
        max_items: Some("1".into()),
        ..Default::default()
    };
    debug!("{:?}", request);

    let zone = client.list_hosted_zones_by_name(request).sync()?;
    debug!("{:?}", &zone);
    if zone.is_truncated {
        return Err("Multiple hosted zones returned for dns_name".into());
    }
    let hosted_zone_id = zone.hosted_zones[0]
        .id
        .trim_start_matches("/hostedzone/")
        .to_string();
    debug!("hosted_zone_id: {}", &hosted_zone_id);

    let request = ListResourceRecordSetsRequest {
        hosted_zone_id: hosted_zone_id.clone(),
        ..Default::default()
    };
    debug!("{:?}", request);
    let record_set = client.list_resource_record_sets(request).sync()?;
    debug!("{:?}", &record_set);
    if zone.is_truncated {
        return Err("Record set iteration not implemented".into());
    }
    let record_set = &record_set
        .resource_record_sets
        .iter()
        .filter(|rs| rs.type_ == "A" && rs.name == dns_name)
        .next()
        .ok_or("Record set does not contain an A record for the dns_name")?;

    if let Some(ref records) = record_set.resource_records {
        if records.len() != 1 {
            return Err("Request didn't return 1 resource record for dns_name".into());
        }
        let current_a = &records[0].value;
        debug!("A Record current value: {}", current_a);
        if *current_a == my_ip {
            return Ok(false);
        }
    }

    let request = ChangeResourceRecordSetsRequest {
        hosted_zone_id,
        change_batch: ChangeBatch {
            comment: Some("route53-dyndns A IP Update".into()),
            changes: vec![Change {
                action: "UPSERT".into(),
                resource_record_set: ResourceRecordSet {
                    name: dns_name.into(),
                    type_: "A".into(),
                    resource_records: Some(vec![ResourceRecord {
                        value: my_ip.into(),
                    }]),
                    ttl: Some(900),
                    ..Default::default()
                },
            }],
        },
    };
    debug!("{:?}", request);
    let change_info = client
        .change_resource_record_sets(request)
        .sync()?
        .change_info;
    debug!("{:?}", &change_info);

    match change_info.status.as_str() {
        "INSYNC" => return Ok(true),
        "PENDING" => {}
        _ => return Err("Invalid ChangeInfo Status".into()),
    }

    // Poll pending change until completed or timed out
    let poll_id = change_info.id.trim_start_matches("/change/").to_string();
    let poll_request = GetChangeRequest { id: poll_id };
    let poll_start = SystemTime::now();

    while poll_start.elapsed()? < Duration::from_secs(60) {
        debug!("{:?}", &poll_request);
        let poll_info = client.get_change(poll_request.clone()).sync()?.change_info;
        debug!("{:?}", &poll_info);

        match poll_info.status.as_str() {
            "INSYNC" => return Ok(true),
            "PENDING" => {}
            _ => return Err("Invalid ChangeInfo Status".into()),
        }

        thread::sleep(Duration::from_secs(1));
    }

    Err("Timeout polling for change completion event".into())
}
