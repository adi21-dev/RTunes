//! Deterministic xorshift for spawn jitter (no `rand` dependency).

/// Returns `[0, 1)` pseudo-random; advances `state`.
pub fn xorshift_u01(state: &mut u64) -> f32 {
    *state ^= state.wrapping_shl(13);
    *state ^= state.wrapping_shr(7);
    *state ^= state.wrapping_shl(17);
    ((*state >> 32) as u32 as f32) * (1.0 / u32::MAX as f32)
}
