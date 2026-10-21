//! The editor can cancel admission without dropping an executing worker's owner.
use crate::{
    app::App,
    bridge::Command,
    sketch_services::{Request, effect_tag},
};
use onecad_app::sketch_candidate::SketchCandidateControl;
use onecad_interaction::sketch::{SketchEffect, SketchTag};

pub(crate) struct Pending {
    tag: SketchTag,
    control: SketchCandidateControl,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

impl App {
    pub(crate) fn cancel_sketch_candidate(&mut self, tag: &SketchTag) {
        if let Some(pending) = self
            .sketch_candidate
            .as_ref()
            .filter(|pending| pending.tag == *tag)
        {
            pending.control.cancel();
        }
    }

    pub(crate) fn finish_sketch_candidate(&mut self, tag: Option<&SketchTag>) {
        if self
            .sketch_candidate
            .as_ref()
            .is_some_and(|pending| Some(&pending.tag) == tag)
        {
            self.sketch_candidate = None;
        }
    }

    pub(crate) fn submit_sketch_effect(&mut self, effect: SketchEffect) -> Result<(), String> {
        let request = if matches!(
            effect,
            SketchEffect::Upsert { .. }
                | SketchEffect::SetDimension { .. }
                | SketchEffect::AddConstraints { .. }
        ) {
            let control = SketchCandidateControl::default();
            self.sketch_candidate = Some(Pending {
                tag: effect_tag(&effect).clone(),
                control: control.clone(),
            });
            Request::Candidate { effect, control }
        } else {
            Request::Effect(effect)
        };
        let result = self.bridge.submit(Command::Sketch(Box::new(request)));
        if result.is_err() {
            self.sketch_candidate = None;
        }
        result
    }
}
