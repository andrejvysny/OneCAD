//! Layout shared by the Settings pages: header, captioned sections, label/control
//! rows and the platform metrics that widen touch targets on iPad.
use egui::{Align, Layout, Response, RichText, Stroke, Ui, text::LayoutJob, vec2};
use onecad_widgets::{
    components::{
        lists::{Segment, SegmentedControl},
        tooltip::{self, Tip},
    },
    fonts,
    icon::Icon,
    metrics::{CONTROL, SECONDARY, SECTION, SECTION_TRACKING},
    theme::look,
};

pub const CONTENT_MAX: f32 = 680.0;
/// Space after every row; a section's body ends with it.
pub const ROW_GAP: f32 = 16.0;
const SECTION_GAP: f32 = 26.0;
const LABEL_W: f32 = 220.0;
const LABEL_W_NARROW: f32 = 180.0;
const NARROW_BELOW: f32 = 640.0;

/// Minimum sensed height for custom controls on touch hosts; 0 keeps the design size.
pub fn hit() -> f32 {
    if touch() { 44.0 } else { 0.0 }
}

/// Below this the shortcut table drops its CONTEXT column.
pub fn very_narrow(width: f32) -> bool {
    width < 400.0
}

pub fn touch() -> bool {
    cfg!(target_os = "ios")
}

/// Height of one interactive row or list item.
pub fn interactive_height() -> f32 {
    interactive_height_for(touch())
}

pub fn interactive_height_for(touch: bool) -> f32 {
    if touch { 44.0 } else { 30.0 }
}

pub fn label_width(content_width: f32) -> f32 {
    label_width_for(touch(), content_width)
}

pub fn label_width_for(touch: bool, content_width: f32) -> f32 {
    if touch && content_width < NARROW_BELOW {
        LABEL_W_NARROW
    } else {
        LABEL_W
    }
}

/// "this Mac" or "this iPad" in page copy.
pub fn device() -> &'static str {
    if touch() { "this iPad" } else { "this Mac" }
}

pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    title_block(ui, title, subtitle);
    ui.add_space(SECTION_GAP);
}

/// Title and subtitle without the gap after them.
pub fn title_block(ui: &mut Ui, title: &str, subtitle: &str) {
    let t = look(ui.ctx()).theme;
    ui.label(
        RichText::new(title)
            .size(20.0)
            .family(fonts::semibold())
            .color(t.ink),
    );
    ui.add_space(4.0);
    ui.label(RichText::new(subtitle).size(CONTROL).color(t.ink3));
}

/// Caps title over a hairline, then `body`, which ends with [`ROW_GAP`].
pub fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        // set_max_width sets, not caps: clamp to the column or the section outgrows it.
        ui.set_max_width(ui.available_width().min(CONTENT_MAX));
        caps_title(ui, title);
        ui.add_space(14.0);
        body(ui);
    });
    ui.add_space(SECTION_GAP - ROW_GAP);
}

fn caps_title(ui: &mut Ui, title: &str) {
    let t = look(ui.ctx()).theme;
    let mut job = LayoutJob::default();
    job.append(
        title,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(SECTION, fonts::medium()),
            color: t.ink5,
            extra_letter_spacing: SECTION_TRACKING,
            ..Default::default()
        },
    );
    ui.label(job);
    ui.add_space(8.0);
    let y = ui.cursor().top();
    let width = ui.available_width().min(CONTENT_MAX);
    let x = ui.cursor().left();
    ui.painter()
        .hline(x..=x + width, y, Stroke::new(1.0, t.border_subtle));
}

/// Row with a vertically centred label column and the control(s) beside it.
pub fn row(ui: &mut Ui, label: &str, caption: Option<&str>, control: impl FnOnce(&mut Ui)) {
    labelled(ui, label, caption, Align::Center, control);
}

/// Like [`row`], with the label aligned to the control's top edge (tall controls).
pub fn row_top(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui)) {
    labelled(ui, label, None, Align::Min, control);
}

fn labelled(
    ui: &mut Ui,
    label: &str,
    caption: Option<&str>,
    align: Align,
    control: impl FnOnce(&mut Ui),
) {
    let t = look(ui.ctx()).theme;
    let height = interactive_height();
    let width = ui.available_width();
    if narrow(width) {
        stacked(ui, label, caption, control);
        return;
    }
    let label_w = label_width(width);
    ui.allocate_ui_with_layout(vec2(width, height), Layout::left_to_right(align), |ui| {
        ui.set_min_height(height);
        ui.spacing_mut().item_spacing.x = 24.0;
        ui.spacing_mut().interact_size.y = height;
        if caption.is_none() && align == Align::Center {
            // Layout main-axis centring does not apply to a lone label here.
            let (rect, _) = ui.allocate_exact_size(vec2(label_w, height), egui::Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::new(CONTROL, fonts::medium()),
                t.ink,
            );
            control(ui);
            return;
        }
        ui.allocate_ui_with_layout(
            vec2(label_w, height),
            Layout::top_down(Align::Min).with_main_align(align),
            |ui| {
                ui.set_min_size(vec2(label_w, height));
                ui.spacing_mut().item_spacing.y = 2.0;
                if align == Align::Min {
                    ui.add_space(4.0);
                }
                ui.label(
                    RichText::new(label)
                        .size(CONTROL)
                        .family(fonts::medium())
                        .color(t.ink),
                );
                if let Some(caption) = caption {
                    ui.label(RichText::new(caption).size(SECONDARY).color(t.ink5));
                }
            },
        );
        control(ui);
    });
    ui.add_space(ROW_GAP);
}

/// Narrow columns (iPad portrait, small windows) stack each row: label over control.
pub fn narrow(width: f32) -> bool {
    width < NARROW_BELOW
}

fn stacked(ui: &mut Ui, label: &str, caption: Option<&str>, control: impl FnOnce(&mut Ui)) {
    let t = look(ui.ctx()).theme;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(label)
                .size(CONTROL)
                .family(fonts::medium())
                .color(t.ink),
        );
        if let Some(caption) = caption {
            ui.label(RichText::new(caption).size(SECONDARY).color(t.ink5));
        }
    });
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().interact_size.y = interactive_height();
        control(ui);
    });
    ui.add_space(8.0);
}

/// Muted help line with the info glyph; ends with [`ROW_GAP`].
pub fn note(ui: &mut Ui, text: &str) {
    let t = look(ui.ctx()).theme;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add(Icon::new("help").size(14.0).tint(t.ink5));
        ui.add(egui::Label::new(RichText::new(text).size(SECONDARY).color(t.ink5)).wrap());
    });
    ui.add_space(ROW_GAP);
}

/// Why a control is inert; the tooltip also reaches controls that only look disabled.
pub fn reason(response: Response, name: &str, reason: &str) -> Response {
    tooltip::attach(response, Tip::new(name).reason(Some(reason)))
}

/// Touch height, and in very narrow columns a width that squeezes labels to fit.
fn fitted<'a>(control: SegmentedControl<'a>, available: f32) -> SegmentedControl<'a> {
    let control = control.hit_height(hit());
    if very_narrow(available) {
        control.width(available)
    } else {
        control
    }
}

/// Segmented control over `labels`; returns the newly chosen index.
pub fn segmented(ui: &mut Ui, id: &str, selected: usize, labels: &[&str]) -> Option<usize> {
    let mut now = selected;
    let segments: Vec<Segment> = labels
        .iter()
        .map(|label| Segment { label, icon: None })
        .collect();
    ui.add(fitted(
        SegmentedControl::new(id, &mut now, &segments),
        ui.available_width(),
    ));
    (now != selected).then_some(now)
}

/// Segmented control fixed at `selected`, with the reason it cannot change.
pub fn segmented_disabled(
    ui: &mut Ui,
    id: &str,
    selected: usize,
    labels: &[&str],
    name: &str,
    why: &str,
) {
    let mut now = selected;
    let segments: Vec<Segment> = labels
        .iter()
        .map(|label| Segment { label, icon: None })
        .collect();
    let out = ui.add_enabled_ui(false, |ui| {
        let width = ui.available_width();
        ui.add(fitted(
            SegmentedControl::new(id, &mut now, &segments),
            width,
        ))
    });
    reason(out.inner, name, why);
}

/// Read-only value text.
pub fn value(ui: &mut Ui, text: &str) {
    let t = look(ui.ctx()).theme;
    ui.label(RichText::new(text).size(CONTROL).color(t.ink3));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_metrics_widen_targets_and_narrow_labels() {
        assert_eq!(interactive_height_for(false), 30.0);
        assert_eq!(interactive_height_for(true), 44.0);
        assert_eq!(label_width_for(false, 400.0), 220.0);
        assert_eq!(label_width_for(true, 639.0), 180.0);
        assert_eq!(label_width_for(true, 640.0), 220.0);
    }

    #[test]
    fn columns_under_640_are_narrow() {
        assert!(narrow(639.0) && !narrow(640.0));
        assert!(very_narrow(399.0) && !very_narrow(400.0));
    }

    #[test]
    fn page_copy_names_the_device() {
        assert!(device().starts_with("this "));
    }
}
