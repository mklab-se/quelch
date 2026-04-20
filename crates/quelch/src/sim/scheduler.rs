//! Burst-aware Poisson activity scheduler. Calls into jira_gen/confluence_gen
//! at varying intervals with occasional burst and lull modes.

use anyhow::Result;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::time::{Duration, Instant};

use super::{confluence_gen, jira_gen};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Burst,
    Lull,
}

pub async fn run(
    base: String,
    seed: Option<u64>,
    rate_multiplier: f64,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let mut rng = match seed {
        Some(s) => StdRng::seed_from_u64(s),
        None => StdRng::from_entropy(),
    };
    let client = reqwest::Client::new();
    let mut mode = Mode::Normal;
    let mut mode_until = Instant::now();

    while !cancel.is_cancelled() {
        if Instant::now() >= mode_until {
            mode = next_mode(&mut rng);
            mode_until = Instant::now()
                + Duration::from_secs(match mode {
                    Mode::Burst => rng.gen_range(30..=60),
                    Mode::Lull => rng.gen_range(60..=90),
                    Mode::Normal => rng.gen_range(30..=60),
                });
        }

        let dwell_ms = match mode {
            Mode::Burst => rng.gen_range(100..=500),
            Mode::Lull => rng.gen_range(5_000..=15_000),
            Mode::Normal => rng.gen_range(2_000..=8_000),
        };
        let scaled = (dwell_ms as f64 / rate_multiplier).max(10.0) as u64;
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(scaled)) => {}
            _ = cancel.cancelled() => break,
        }

        // 65% Jira / 35% Confluence.
        let pick_jira = rng.r#gen::<f32>() < 0.65;
        let res = if pick_jira {
            jira_gen::mutate(&client, &base, &mut rng).await
        } else {
            confluence_gen::mutate(&client, &base, &mut rng).await
        };
        if let Err(e) = res {
            tracing::warn!(error = %e, "sim: mutation failed (continuing)");
        }
    }
    Ok(())
}

fn next_mode(rng: &mut StdRng) -> Mode {
    let roll: f32 = rng.r#gen();
    if roll < 0.30 {
        Mode::Burst
    } else if roll < 0.50 {
        Mode::Lull
    } else {
        Mode::Normal
    }
}
