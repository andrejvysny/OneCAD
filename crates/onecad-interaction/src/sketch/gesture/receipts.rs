use super::*;

impl SketchGestureInteraction {
    pub(super) fn ready(
        &mut self,
        tag: GestureTag,
        worker_id: u64,
        preview: Box<Sketch>,
    ) -> Vec<GestureEffect> {
        if !self.pending(&tag, GesturePhase::PendingBegin) {
            return Vec::new();
        }
        if worker_id == 0
            || !super::preview::checked(&self.gesture.as_ref().unwrap().baseline, &preview)
        {
            self.error = Some("Gesture worker returned an invalid identity or preview".into());
            return self.cancel();
        }
        let gesture = self.gesture.as_mut().unwrap();
        gesture.worker_id = Some(worker_id);
        gesture.preview = Some(preview);
        gesture.phase = GesturePhase::Ready;
        self.flush()
    }

    pub(super) fn updated(
        &mut self,
        tag: GestureTag,
        worker_id: u64,
        worker_sequence: u64,
        preview: Box<Sketch>,
    ) -> Vec<GestureEffect> {
        if !self.pending(&tag, GesturePhase::PendingUpdate) {
            return Vec::new();
        }
        let gesture = self.gesture.as_ref().unwrap();
        if gesture.worker_id != Some(worker_id) || worker_sequence <= gesture.worker_sequence {
            return Vec::new();
        }
        if !super::preview::checked(&gesture.baseline, &preview) {
            self.error =
                Some("Gesture preview changed accepted identities or invalid geometry".into());
            return self.cancel();
        }
        let gesture = self.gesture.as_mut().unwrap();
        gesture.worker_sequence = worker_sequence;
        gesture.preview = Some(preview);
        gesture.phase = GesturePhase::Ready;
        self.flush()
    }

    pub(super) fn completed(&mut self, tag: GestureTag, worker_id: u64, moved: bool) {
        if self.pending(&tag, GesturePhase::PendingEnd)
            && self
                .gesture
                .as_ref()
                .is_some_and(|gesture| gesture.worker_id == Some(worker_id))
        {
            self.moved = Some(moved);
            self.gesture = None;
        }
    }

    pub(super) fn cancelled(&mut self, tag: GestureTag) {
        if self.pending(&tag, GesturePhase::Cancelling) {
            self.gesture = None;
        }
    }

    pub(super) fn failed(&mut self, tag: GestureTag, message: &str) -> Vec<GestureEffect> {
        if self
            .gesture
            .as_ref()
            .is_none_or(|gesture| gesture.tag != tag)
        {
            return Vec::new();
        }
        self.refuse(message);
        if self.phase() == Some(GesturePhase::Cancelling) {
            self.gesture = None;
            return Vec::new();
        }
        self.cancel()
    }

    fn pending(&self, tag: &GestureTag, phase: GesturePhase) -> bool {
        self.gesture
            .as_ref()
            .is_some_and(|gesture| gesture.phase == phase && gesture.tag == *tag)
    }
}
