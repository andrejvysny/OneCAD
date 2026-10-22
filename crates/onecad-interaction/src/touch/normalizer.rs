//! INPUT.md normalizer rules 1–7: per-contact chronology, coalesced dedupe,
//! prediction replacement, estimation updates, staleness, generations and bounds.
use super::{
    mask, CancelReason, ContactId, ContactKind, Normalized, Phase, Point, Pose, Provenance,
    RawSample, Sample, ESTIMATION_RING, MAX_BATCH, MAX_CONTACTS, MAX_PENDING,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Contacts whose estimation records outlive the contact (UIKit refines
/// Pencil force and angles after the touch ended).
const RETAINED_RINGS: usize = 32;
/// Contacts cancelled by a generation change or `cancel_all` that may still
/// report their own end.
const RETIRED_MAX: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitError;

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "batch exceeds {MAX_BATCH} samples")
    }
}

/// Counters for dropped and merged input; part of the trace digest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormalizerStats {
    pub invalid: u64,
    pub duplicate_coalesced: u64,
    pub stale: u64,
    pub unknown_contact: u64,
    pub old_generation: u64,
    pub refused_contacts: u64,
    pub estimation_matched: u64,
    pub estimation_unmatched: u64,
    pub merged_moves: u64,
}

struct Live {
    kind: ContactKind,
    last: f64,
    predicted: bool,
}

#[derive(Default)]
pub struct Normalizer {
    live: BTreeMap<ContactId, Live>,
    retired: VecDeque<ContactId>,
    rings: VecDeque<(ContactId, VecDeque<(u64, Pose)>)>,
    newest_generation: u64,
    pending: Vec<Normalized>,
    stats: NormalizerStats,
}

impl Normalizer {
    /// Converts and submits host samples; invalid ones are counted and dropped.
    pub fn submit_raw(&mut self, raw: &[RawSample], generation: u64) -> Result<bool, SubmitError> {
        if raw.len() > MAX_BATCH {
            return Err(SubmitError);
        }
        let samples: Vec<Sample> = raw.iter().filter_map(Sample::from_raw).collect();
        self.stats.invalid += (raw.len() - samples.len()) as u64;
        self.submit(&samples, generation)
    }

    /// Normalizes one host batch into the pending queue. `Ok(true)` when the
    /// pending bound forced intermediate movement to merge.
    pub fn submit(&mut self, samples: &[Sample], generation: u64) -> Result<bool, SubmitError> {
        if samples.len() > MAX_BATCH {
            return Err(SubmitError);
        }
        if generation > self.newest_generation {
            self.newest_generation = generation;
            self.cancel_live(CancelReason::Generation);
        }
        let stale_batch = generation < self.newest_generation;
        let mut touched = HashSet::new();
        for sample in chronological(samples, &mut self.stats) {
            if !sample.phase.hover() {
                touched.insert(sample.contact);
            }
            self.sample(sample, stale_batch);
        }
        for update in samples
            .iter()
            .filter(|s| s.provenance == Provenance::EstimationUpdate)
        {
            self.estimation(update);
        }
        if !stale_batch {
            self.predictions(samples, &touched);
        }
        Ok(self.enforce_bound())
    }

    /// Cancels every live contact (system gesture, scene inactive, text focus,
    /// document switch); their later samples are dropped.
    pub fn cancel_all(&mut self, reason: CancelReason) {
        self.cancel_live(reason);
    }

    pub fn drain(&mut self) -> Vec<Normalized> {
        std::mem::take(&mut self.pending)
    }

    pub fn stats(&self) -> NormalizerStats {
        self.stats
    }

    pub fn live_contacts(&self) -> usize {
        self.live.len()
    }

    /// The retained pose of a delivered sample, after estimation updates.
    pub fn pose(&self, contact: ContactId, estimation_index: u64) -> Option<Pose> {
        let (_, ring) = self.rings.iter().find(|(id, _)| *id == contact)?;
        ring.iter()
            .find(|(index, _)| *index == estimation_index)
            .map(|(_, pose)| *pose)
    }

    fn cancel_live(&mut self, reason: CancelReason) {
        for (contact, live) in std::mem::take(&mut self.live) {
            if live.predicted {
                self.clear_prediction(contact);
            }
            self.pending.push(Normalized::Cancel { contact, reason });
            self.retire(contact);
        }
    }

    fn retire(&mut self, contact: ContactId) {
        if self.retired.len() == RETIRED_MAX {
            self.retired.pop_front();
        }
        self.retired.push_back(contact);
    }

    fn sample(&mut self, sample: Sample, stale_batch: bool) {
        if sample.phase.hover() {
            if stale_batch {
                self.stats.old_generation += 1;
            } else {
                self.pending.push(Normalized::Hover(sample));
            }
            return;
        }
        if let Some(index) = self.retired.iter().position(|id| *id == sample.contact) {
            // A host may reuse an id whose terminal never arrived: a current
            // `began` starts a new contact instead of being lost with the old one.
            let reused = sample.phase == Phase::Began && !stale_batch;
            if reused || sample.phase.terminal() {
                self.retired.remove(index);
            }
            if !reused {
                // A retired contact's later samples are dropped; its end closes it.
                self.stats.old_generation += 1;
                return;
            }
        }
        if stale_batch {
            self.stale_generation(sample);
            return;
        }
        match sample.phase {
            Phase::Began => self.began(sample),
            Phase::Moved | Phase::Stationary => self.moved(sample),
            _ => self.ended(sample),
        }
    }

    /// A batch captured under an older view: positions are in the wrong frame,
    /// so nothing is delivered and a live contact's end becomes a cancellation.
    fn stale_generation(&mut self, sample: Sample) {
        self.stats.old_generation += 1;
        if sample.phase.terminal() && self.live.contains_key(&sample.contact) {
            self.close(sample.contact);
            self.pending.push(Normalized::Cancel {
                contact: sample.contact,
                reason: CancelReason::Generation,
            });
        }
    }

    fn began(&mut self, sample: Sample) {
        if self.live.contains_key(&sample.contact) {
            self.stats.stale += 1;
            return;
        }
        if self.live.len() >= MAX_CONTACTS {
            self.stats.refused_contacts += 1;
            return;
        }
        self.live.insert(
            sample.contact,
            Live {
                kind: sample.kind,
                last: sample.timestamp,
                predicted: false,
            },
        );
        self.deliver(sample);
    }

    fn moved(&mut self, sample: Sample) {
        let Some(live) = self.live.get_mut(&sample.contact) else {
            self.stats.unknown_contact += 1;
            return;
        };
        if sample.timestamp < live.last || live.kind != sample.kind {
            self.stats.stale += 1;
            return;
        }
        live.last = sample.timestamp;
        self.deliver(sample);
    }

    fn ended(&mut self, sample: Sample) {
        if !self.live.contains_key(&sample.contact) {
            self.stats.unknown_contact += 1;
            return;
        }
        self.close(sample.contact);
        self.deliver(sample);
    }

    fn close(&mut self, contact: ContactId) {
        if self
            .live
            .remove(&contact)
            .is_some_and(|live| live.predicted)
        {
            self.clear_prediction(contact);
        }
    }

    fn clear_prediction(&mut self, contact: ContactId) {
        self.pending.push(Normalized::Prediction {
            contact,
            points: Vec::new(),
        });
    }

    fn deliver(&mut self, sample: Sample) {
        if let Some(index) = sample.estimation_index {
            self.record(sample.contact, index, sample.pose);
        }
        self.pending.push(Normalized::Contact(sample));
    }

    fn record(&mut self, contact: ContactId, index: u64, pose: Pose) {
        let slot = match self.rings.iter().position(|(id, _)| *id == contact) {
            Some(slot) => slot,
            None => {
                if self.rings.len() == RETAINED_RINGS {
                    self.rings.pop_front();
                }
                self.rings.push_back((contact, VecDeque::new()));
                self.rings.len() - 1
            }
        };
        let ring = &mut self.rings[slot].1;
        if ring.len() == ESTIMATION_RING {
            ring.pop_front();
        }
        ring.push_back((index, pose));
    }

    /// Refines optional properties of a retained sample; never a position or event.
    fn estimation(&mut self, update: &Sample) {
        let entry = update.estimation_index.and_then(|index| {
            let (_, ring) = self
                .rings
                .iter_mut()
                .find(|(id, _)| *id == update.contact)?;
            ring.iter_mut().find(|(i, _)| *i == index)
        });
        let Some((_, pose)) = entry else {
            self.stats.estimation_unmatched += 1;
            return;
        };
        refine(pose, &update.pose);
        self.stats.estimation_matched += 1;
    }

    /// Each batch that reports a contact replaces its prediction list.
    fn predictions(&mut self, samples: &[Sample], touched: &HashSet<ContactId>) {
        let mut lists: BTreeMap<ContactId, Vec<Point>> = BTreeMap::new();
        for contact in touched {
            lists.insert(*contact, Vec::new());
        }
        for sample in samples
            .iter()
            .filter(|s| s.provenance == Provenance::Predicted)
        {
            if let Some(point) = sample.position {
                lists.entry(sample.contact).or_default().push(point);
            }
        }
        for (contact, points) in lists {
            let Some(live) = self.live.get_mut(&contact) else {
                continue;
            };
            if points.is_empty() && !live.predicted {
                continue;
            }
            live.predicted = !points.is_empty();
            self.pending
                .push(Normalized::Prediction { contact, points });
        }
    }

    /// Merges consecutive disposable events of one contact (keeping the newest)
    /// until the queue fits; began, ended, cancelled and cancellations stay.
    /// Rule 7 forbids dropping those, so a host that stops draining can grow
    /// the queue past `MAX_PENDING` by them alone; hosts drain every frame.
    fn enforce_bound(&mut self) -> bool {
        let mut excess = self.pending.len().saturating_sub(MAX_PENDING);
        if excess == 0 {
            return false;
        }
        let mut keep = vec![true; self.pending.len()];
        let mut runs: HashMap<(Option<ContactId>, u8), usize> = HashMap::new();
        for (index, event) in self.pending.iter().enumerate() {
            match disposable(event) {
                Some(key) => {
                    if let Some(previous) = runs.insert(key, index) {
                        if excess > 0 {
                            keep[previous] = false;
                            excess -= 1;
                            self.stats.merged_moves += 1;
                        }
                    }
                }
                None => {
                    // A non-disposable event ends this contact's runs.
                    let contact = event.contact();
                    for stream in 0..=2 {
                        runs.remove(&(contact, stream));
                    }
                }
            }
        }
        let mut flags = keep.into_iter();
        self.pending.retain(|_| flags.next().unwrap_or(true));
        true
    }
}

/// Rule 1 and 2: actual and coalesced samples (and hover) in timestamp order,
/// stable for equal times, without the coalesced copy of an actual sample.
fn chronological(samples: &[Sample], stats: &mut NormalizerStats) -> Vec<Sample> {
    let key = |s: &Sample| {
        let position = s.position.unwrap_or(Point::new(f64::NAN, f64::NAN));
        (
            s.contact,
            s.timestamp.to_bits(),
            position.x.to_bits(),
            position.y.to_bits(),
        )
    };
    let actual: HashSet<_> = samples
        .iter()
        .filter(|s| s.provenance == Provenance::Actual)
        .map(key)
        .collect();
    let mut ordered = Vec::with_capacity(samples.len());
    for sample in samples {
        match sample.provenance {
            Provenance::Actual => ordered.push(*sample),
            Provenance::Coalesced if actual.contains(&key(sample)) => {
                stats.duplicate_coalesced += 1;
            }
            Provenance::Coalesced => ordered.push(*sample),
            Provenance::Predicted | Provenance::EstimationUpdate => {}
        }
    }
    ordered.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));
    ordered
}

fn disposable(event: &Normalized) -> Option<(Option<ContactId>, u8)> {
    match event {
        Normalized::Contact(s) if matches!(s.phase, Phase::Moved | Phase::Stationary) => {
            Some((Some(s.contact), 0))
        }
        Normalized::Hover(s) if s.phase == Phase::HoverMoved => Some((None, 1)),
        Normalized::Prediction { contact, points } if !points.is_empty() => {
            Some((Some(*contact), 2))
        }
        _ => None,
    }
}

fn refine(pose: &mut Pose, update: &Pose) {
    let fields = [
        (mask::FORCE, &mut pose.force, update.force),
        (mask::ALTITUDE, &mut pose.altitude, update.altitude),
        (mask::AZIMUTH, &mut pose.azimuth, update.azimuth),
        (mask::ROLL, &mut pose.roll, update.roll),
        (mask::HOVER_Z, &mut pose.hover_z, update.hover_z),
    ];
    let mut refined = 0;
    for (bit, field, value) in fields {
        if let Some(value) = value {
            *field = Some(value);
            refined |= bit;
        }
    }
    // A refined value stops being an estimate unless the update says otherwise.
    pose.estimated &= !(refined & !update.estimated);
    if update.max_force.is_some() {
        pose.max_force = update.max_force;
    }
    pose.expects_update = update.expects_update & mask::POSE;
}
