use std::cmp::Reverse;

use rand::{Rng, RngExt};
use strelitzia_core::Channel;

/// Priority tiers are tried in order; retry_index clamps to the last tier so failover never runs out of candidates.
pub fn select_channel<'a>(
    channels: &'a [Channel],
    retry_index: usize,
    rng: &mut impl Rng,
) -> Option<&'a Channel> {
    let mut enabled: Vec<&Channel> = channels.iter().filter(|channel| channel.enabled).collect();
    if enabled.is_empty() {
        return None;
    }

    enabled.sort_by_key(|channel| Reverse(channel.priority));

    let mut tier_start = 0;
    let mut tier_index = 0;
    let selected_tier = loop {
        let priority = enabled[tier_start].priority;
        let mut tier_end = tier_start + 1;
        while tier_end < enabled.len() && enabled[tier_end].priority == priority {
            tier_end += 1;
        }

        let is_last_tier = tier_end == enabled.len();
        if tier_index == retry_index || is_last_tier {
            break &enabled[tier_start..tier_end];
        }

        tier_index += 1;
        tier_start = tier_end;
    };

    Some(weighted_pick(selected_tier, rng))
}

// Selection is proportional to weight; zero is treated as one so the channel stays selectable.
fn weighted_pick<'a>(tier: &[&'a Channel], rng: &mut impl Rng) -> &'a Channel {
    if let [only] = tier {
        return only;
    }

    let total_weight: u64 = tier
        .iter()
        .map(|channel| u64::from(channel.weight.max(1)))
        .sum();
    let mut ticket = rng.random_range(0..total_weight);

    for channel in tier {
        let weight = u64::from(channel.weight.max(1));
        if ticket < weight {
            return channel;
        }
        ticket -= weight;
    }

    // `ticket` is always below `total_weight`, so the loop above returns.
    tier.last().copied().expect("tier is never empty")
}
