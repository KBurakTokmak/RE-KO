//! Native balance notifications for transactions outside shop and loot replies.
pub(crate) fn change(before: u32, after: u32) -> Vec<u8> {
    let mut packet = vec![0x4a, if after >= before { 1 } else { 2 }];
    packet.extend(before.abs_diff(after).to_le_bytes());
    packet.extend(after.to_le_bytes());
    packet
}
