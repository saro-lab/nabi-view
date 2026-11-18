//! Direction-aware working sets. Mobile keeps a small decoded set in memory
//! and prepares the rest of ±10 pages on disk only; saved desktop cache
//! settings (image count, wrapping) remain unchanged.
use crate::core::media::Urgency;

const NEIGHBORS: usize = 10;
const FIRST_THUMBNAILS: usize = 20;
/// Pages ahead counted as near-term display work after the lead.
const AHEAD: usize = 5;
/// Mobile memory keeps at most this many decoded pages ahead and behind.
const MEMORY_AHEAD: usize = 5;
const MEMORY_BEHIND: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub index: usize,
    pub urgency: Urgency,
    /// Decode into the disk cache without keeping a texture.
    pub disk_only: bool,
}

/// Where the current view sits and how far its working set may reach.
#[derive(Clone, Copy)]
pub(super) struct Position {
    pub len: usize,
    pub first: usize,
    pub last: usize,
    pub backward: bool,
    pub mobile: bool,
    /// Decoded pages kept in memory, current pages included.
    pub limit: usize,
}

impl Position {
    /// `distance` pages past the visible span, ahead in the reading direction
    /// or behind it. Desktop wraps; mobile stops at the ends.
    fn step(&self, distance: usize, ahead: bool) -> Option<usize> {
        let forward = ahead != self.backward;
        if self.mobile {
            if forward {
                self.last.checked_add(distance).filter(|&i| i < self.len)
            } else {
                self.first.checked_sub(distance)
            }
        } else {
            let distance = distance % self.len;
            Some(if forward {
                (self.last + distance) % self.len
            } else {
                (self.first + self.len - distance) % self.len
            })
        }
    }
}

/// Speculative display work in priority order: the next page or spread, the
/// one behind, the next few ahead, then the rest alternating by distance.
pub(super) fn display_targets(at: Position) -> Vec<Target> {
    if at.len == 0 || at.first > at.last || at.last >= at.len {
        return Vec::new();
    }
    let lead = at.last - at.first + 1;
    let reach = if at.mobile { NEIGHBORS } else { at.len };
    // Lazy: a desktop folder may hold many thousands of pages.
    let order = (1..=lead)
        .map(|d| (d, true, Urgency::Lead))
        .chain((1..=lead).map(|d| (d, false, Urgency::Opposite)))
        .chain((lead + 1..=AHEAD.max(lead)).map(|d| (d, true, Urgency::Ahead)))
        .chain((1..=reach).flat_map(|d| [(d, true, Urgency::Ahead), (d, false, Urgency::Ahead)]));

    let mut seen: std::collections::HashSet<usize> = (at.first..=at.last).collect();
    let mut memory = seen.len();
    let mut targets = Vec::new();
    for (distance, ahead, urgency) in order {
        if distance > reach {
            continue;
        }
        let Some(index) = at.step(distance, ahead) else {
            continue;
        };
        if !seen.insert(index) {
            continue;
        }
        let near = if ahead {
            distance <= MEMORY_AHEAD
        } else {
            distance <= MEMORY_BEHIND
        };
        let in_memory = (!at.mobile || near) && memory < at.limit;
        if !in_memory && !at.mobile {
            // Desktop keeps its configured image count and nothing on disk only.
            break;
        }
        memory += usize::from(in_memory);
        targets.push(Target {
            index,
            urgency,
            disk_only: !in_memory,
        });
    }
    targets
}

/// Mobile thumbnails around the focus, nearest first and ahead before behind.
pub(super) fn thumbnails(len: usize, focus: usize, backward: bool) -> Vec<usize> {
    if len == 0 || focus >= len {
        return Vec::new();
    }
    if focus == 0 {
        return (0..FIRST_THUMBNAILS.min(len)).collect();
    }
    let mut order = vec![focus];
    for distance in 1..=NEIGHBORS {
        let later = focus.checked_add(distance).filter(|&i| i < len);
        let earlier = focus.checked_sub(distance);
        let (ahead, behind) = if backward {
            (earlier, later)
        } else {
            (later, earlier)
        };
        order.extend(ahead);
        order.extend(behind);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(len: usize, first: usize, last: usize, backward: bool, mobile: bool) -> Position {
        Position {
            len,
            first,
            last,
            backward,
            mobile,
            limit: 1000,
        }
    }

    fn indices(targets: &[Target]) -> Vec<usize> {
        targets.iter().map(|t| t.index).collect()
    }

    #[test]
    fn mobile_thumbnails_clip_at_edges_and_follow_direction() {
        assert_eq!(thumbnails(1000, 0, false), (0..20).collect::<Vec<_>>());
        assert_eq!(thumbnails(7, 0, false), (0..7).collect::<Vec<_>>());
        let forward = thumbnails(1000, 100, false);
        assert_eq!(&forward[..5], [100, 101, 99, 102, 98]);
        assert_eq!(forward.len(), 21);
        assert_eq!(&thumbnails(1000, 100, true)[..3], [100, 99, 101]);
        assert_eq!(thumbnails(1000, 999, false).len(), 11);
        assert!(thumbnails(0, 0, false).is_empty());
    }

    #[test]
    fn next_page_leads_then_one_behind_then_further_ahead() {
        let forward = display_targets(at(1000, 100, 100, false, true));
        assert_eq!(&indices(&forward)[..6], [101, 99, 102, 103, 104, 105]);
        assert_eq!(forward[0].urgency, Urgency::Lead);
        assert_eq!(forward[1].urgency, Urgency::Opposite);
        assert_eq!(forward[2].urgency, Urgency::Ahead);
        let backward = display_targets(at(1000, 100, 100, true, true));
        assert_eq!(&indices(&backward)[..3], [99, 101, 98]);
        // A spread leads with the whole next spread.
        let spread = display_targets(at(1000, 100, 101, false, true));
        assert_eq!(&indices(&spread)[..4], [102, 103, 99, 98]);
        assert_eq!(spread[1].urgency, Urgency::Lead);
    }

    #[test]
    fn mobile_keeps_five_ahead_two_behind_in_memory_and_the_rest_on_disk() {
        let targets = display_targets(at(1000, 100, 100, false, true));
        assert_eq!(targets.len(), 20, "±10 around the current page");
        let memory: Vec<_> = targets
            .iter()
            .filter(|t| !t.disk_only)
            .map(|t| t.index)
            .collect();
        assert_eq!(memory, [101, 99, 102, 103, 104, 105, 98]);
        assert!(targets.iter().all(|t| t.index.abs_diff(100) <= 10));
        // Edges clip instead of wrapping.
        let end = display_targets(at(1000, 999, 999, false, true));
        assert_eq!(end.len(), 10);
        assert!(end.iter().all(|t| t.index < 999));
        // A one-image memory setting leaves only disk work.
        let tight = display_targets(Position {
            limit: 1,
            ..at(1000, 100, 100, false, true)
        });
        assert!(tight.iter().all(|t| t.disk_only));
    }

    #[test]
    fn desktop_wraps_within_its_image_count_and_never_uses_disk_only() {
        let targets = display_targets(Position {
            limit: 5,
            ..at(10, 9, 9, false, false)
        });
        assert_eq!(indices(&targets), [0, 8, 1, 2]);
        assert!(targets.iter().all(|t| !t.disk_only));
        let all = display_targets(at(10, 0, 0, false, false));
        assert_eq!(all.len(), 9);
    }
}
