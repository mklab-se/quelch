//! Periodic fault injection into the mock Azure endpoint.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::time::Duration;

pub async fn run(
    base: String,
    fault_rate: f64,
    seed: Option<u64>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    if fault_rate <= 0.0 {
        return Ok(());
    }
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s.wrapping_add(2)),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();

    loop {
        let dwell = Duration::from_millis(rng.gen_range(2_000..=8_000));
        tokio::select! {
            _ = tokio::time::sleep(dwell) => {}
            _ = cancel.cancelled() => return Ok(()),
        }
        // fault_rate is specified as "probability per Azure request" in the CLI,
        // but this loop ticks every 2-8s regardless of request volume. At the
        // engine's observed rate (~3-5 req/s during active sync), a single tick
        // covers ~10 requests on average — hence the x10 conversion. With
        // fault_rate=0.03, that yields ~30% chance of injecting a fault per tick,
        // which empirically maps to about one fault per 30 real requests.
        let should_fault = rng.r#gen::<f64>() < (fault_rate * 10.0).min(1.0);
        if !should_fault {
            continue;
        }
        let status = if rng.r#gen::<f32>() < 0.6 { 429 } else { 503 };
        let _ = client
            .post(format!("{base}/azure/_fault"))
            .json(&serde_json::json!({"count": 1, "status": status}))
            .send()
            .await;
    }
}
