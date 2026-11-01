//! Display-only rounding; geometry keeps full precision.

pub(crate) fn format_measure(value: f64) -> String {
    format_measure_to(value, 2)
}

pub(crate) fn format_measure_to(value: f64, max_decimals: u8) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let scale = 10f64.powi(i32::from(max_decimals));
    let text = format!(
        "{:.*}",
        usize::from(max_decimals),
        (value * scale).round() / scale
    );
    // Integers keep their trailing zeros ("100"); only a fraction is trimmed.
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text.is_empty() || text == "-0" || text == "-" {
        "0".into()
    } else {
        text.into()
    }
}

/// Sweeps and signed angles read as a positive turn.
pub(crate) fn format_angle_degrees(deg: f64) -> String {
    format_angle_degrees_to(deg, 2)
}

pub(crate) fn format_angle_degrees_to(deg: f64, max_decimals: u8) -> String {
    if !deg.is_finite() {
        return "0".into();
    }
    let wrapped = deg.rem_euclid(360.0);
    // A value that rounds up to a full turn reads as 0, not "360".
    let half_step = 0.5 / 10f64.powi(i32::from(max_decimals));
    format_measure_to(
        if wrapped >= 360.0 - half_step {
            0.0
        } else {
            wrapped
        },
        max_decimals,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_round_to_two_decimals_and_trim() {
        assert_eq!(format_measure(1.7949439391), "1.79");
        assert_eq!(format_measure(5.0), "5");
        assert_eq!(format_measure(9.999999999999998), "10");
        assert_eq!(format_measure(2.5), "2.5");
        assert_eq!(format_measure(-0.001), "0");
        assert_eq!(format_measure(-1.256), "-1.26");
        assert_eq!(format_measure(100.0), "100");
        assert_eq!(format_measure(f64::NAN), "0");
        assert_eq!(format_measure(f64::INFINITY), "0");
    }

    #[test]
    fn decimals_variants_round_trim_and_keep_zero_rules() {
        assert_eq!(format_measure_to(1.7949, 3), "1.795");
        assert_eq!(format_measure_to(1.5, 3), "1.5");
        assert_eq!(format_measure_to(1.7949, 0), "2");
        assert_eq!(format_measure_to(100.0, 0), "100");
        assert_eq!(format_measure_to(-0.4, 0), "0");
        assert_eq!(format_measure_to(-0.0004, 3), "0");
        assert_eq!(format_measure_to(f64::NAN, 3), "0");
        assert_eq!(format_angle_degrees_to(-90.0, 0), "270");
        assert_eq!(format_angle_degrees_to(359.6, 0), "0");
        assert_eq!(format_angle_degrees_to(359.9996, 3), "0");
        assert_eq!(format_angle_degrees_to(12.3456, 3), "12.346");
        assert_eq!(format_angle_degrees_to(f64::INFINITY, 3), "0");
    }

    #[test]
    fn angles_normalize_into_a_positive_turn() {
        assert_eq!(format_angle_degrees(-90.0), "270");
        assert_eq!(format_angle_degrees(450.0), "90");
        assert_eq!(format_angle_degrees(-1e-12), "0");
        assert_eq!(format_angle_degrees(359.999), "0");
        assert_eq!(format_angle_degrees(90.0), "90");
    }
}
